//! Game audio on Linux: a capture of one application's sound from the PipeWire graph, handed
//! to whoever started it buffer by buffer (`app_audio` encodes and sends it).
//!
//! Every application playing sound is a node in the graph (media class `Stream/Output/Audio`)
//! with an output port per channel. The source opens a capture stream of its own with
//! autoconnect off, so the session manager leaves it alone, and links the application's output
//! ports to its own input ports through the link factory, one link per channel. An input port
//! mixes whatever is linked to it, so every stream of the application arrives as one signal,
//! which the stream's adapter converts to 48 kHz stereo float, and each buffer goes to the
//! capture's callback. The application keeps playing to its sink; the links fan its output
//! out. This is the same mechanism the desktop's own per-application audio tools
//! use, and it needs only a PipeWire socket, not a particular session manager.
//!
//! The application is named by process id and name (`application.process.id` and
//! `application.name` on its nodes). The nodes with that process id are the target while any
//! exist; when none does, the nodes with that name are, which follows a game across a restart.
//! Nodes come and go while the capture runs (games open a stream per sound device, browsers
//! one per tab), and the registry listener relinks as they do.
//!
//! A `Target` is `pid` (the process id, 0 for none) and `application` (the name), as the
//! settings of a `startAudio` request carry it; `playing_applications` lists what can be
//! captured right now in the same vocabulary.

use crate::AudioTarget;
use pipewire as pw;
use pw::context::ContextRc;
use pw::core::CoreRc;
use pw::link::Link;
use pw::main_loop::MainLoopRc;
use pw::node::{Node, NodeChangeMask, NodeListener};
use pw::properties::properties;
use pw::registry::{GlobalObject, Registry, RegistryRc};
use pw::spa;
use pw::stream::{StreamFlags, StreamRc, StreamState};
use pw::types::ObjectType;
use serde::Deserialize;
use spa::utils::dict::DictRef;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

const RATE: u32 = 48_000;
const CHANNELS: u32 = 2;
const OUTPUT_STREAM_CLASS: &str = "Stream/Output/Audio";
/// How long `create` waits for the capture thread to reach PipeWire before giving up.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// What the capture captures, as the settings of a `startAudio` request name it.
#[derive(Clone, Debug, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct Target {
    pid: Option<u32>,
    application: String,
}

impl Target {
    fn settings(&self) -> serde_json::Value {
        let mut settings = serde_json::Map::new();
        settings.insert("application".into(), self.application.clone().into());
        if let Some(pid) = self.pid {
            settings.insert("pid".into(), pid.into());
        }
        serde_json::Value::Object(settings)
    }

    /// The target `settings` name, a JSON object with `application` and `pid`; none, an empty
    /// target, which captures nothing until retargeted.
    pub fn from_settings(settings: Option<&str>) -> Result<Target, String> {
        let Some(json) = settings else {
            return Ok(Target::default());
        };
        let mut target: Target =
            serde_json::from_str(json).map_err(|e| format!("audio settings: {e}"))?;
        target.pid = target.pid.filter(|pid| *pid != 0);
        Ok(target)
    }
}

/// An application's playback stream, as the registry describes it.
#[derive(Clone, Debug, PartialEq)]
struct NodeInfo {
    application: String,
    pid: Option<u32>,
}

/// An audio port of some node. Monitor ports and ports without an audio channel are not kept.
#[derive(Clone, Debug, PartialEq)]
struct PortInfo {
    node: u32,
    input: bool,
    channel: String,
}

fn is_output_stream(global: &GlobalObject<&DictRef>) -> bool {
    global.type_ == ObjectType::Node
        && global
            .props
            .is_some_and(|props| props.get("media.class") == Some(OUTPUT_STREAM_CLASS))
}

/// What a node's properties say of its application. The registry announces only a node's
/// headline properties (its name, class, and application name); the process id arrives with
/// the node's own info once it is bound, so this runs on both.
fn node_info(props: &DictRef) -> NodeInfo {
    let application = props
        .get("application.name")
        .or_else(|| props.get("node.name"))
        .or_else(|| props.get("application.process.binary"))
        .unwrap_or("")
        .to_string();
    let pid = props
        .get("application.process.id")
        .and_then(|pid| pid.parse().ok())
        .filter(|pid| *pid != 0);
    NodeInfo { application, pid }
}

/// Binds an application stream's node so its full properties arrive, handing each reading to
/// `on_info`. An info event carries only what changed, so one about the node's state alone is
/// not a reading. The proxy and listener live as long as the node is watched.
fn watch_node(
    registry: &Registry,
    global: &GlobalObject<&DictRef>,
    on_info: impl Fn(NodeInfo) + 'static,
) -> Option<(Node, NodeListener)> {
    let node: Node = registry.bind(global).ok()?;
    let listener = node
        .add_listener_local()
        .info(move |info| {
            if !info.change_mask().contains(NodeChangeMask::PROPS) {
                return;
            }
            if let Some(props) = info.props() {
                on_info(node_info(props));
            }
        })
        .register();
    Some((node, listener))
}

fn audio_port(global: &GlobalObject<&DictRef>) -> Option<PortInfo> {
    if global.type_ != ObjectType::Port {
        return None;
    }
    let props = global.props?;
    if props.get("port.monitor") == Some("true") {
        return None;
    }
    let channel = props.get("audio.channel")?;
    Some(PortInfo {
        node: props.get("node.id")?.parse().ok()?,
        input: props.get("port.direction")? == "in",
        channel: channel.to_string(),
    })
}

// ---------------------------------------------------------------------------------------------
// Listing

/// The applications with a playback stream open right now, one entry per process (or per name
/// when a stream carries no process id), sorted by name.
pub fn playing_applications() -> Result<Vec<AudioTarget>, String> {
    pw::init();
    let mainloop = MainLoopRc::new(None).map_err(|e| format!("PipeWire loop: {e}"))?;
    let context = ContextRc::new(&mainloop, None).map_err(|e| format!("PipeWire context: {e}"))?;
    let core = context
        .connect_rc(None)
        .map_err(|e| format!("could not connect to PipeWire: {e}"))?;
    let registry = core
        .get_registry_rc()
        .map_err(|e| format!("PipeWire registry: {e}"))?;
    let nodes: Rc<RefCell<HashMap<u32, NodeInfo>>> = Rc::new(RefCell::new(HashMap::new()));
    let watched: Rc<RefCell<Vec<(Node, NodeListener)>>> = Rc::new(RefCell::new(Vec::new()));
    let _registry_listener = registry
        .add_listener_local()
        .global({
            let nodes = nodes.clone();
            let watched = watched.clone();
            let registry = registry.clone();
            move |global| {
                if !is_output_stream(global) {
                    return;
                }
                let id = global.id;
                if let Some(props) = global.props {
                    nodes.borrow_mut().insert(id, node_info(props));
                }
                let watch = watch_node(&registry, global, {
                    let nodes = nodes.clone();
                    move |info| {
                        nodes.borrow_mut().insert(id, info);
                    }
                });
                if let Some(watch) = watch {
                    watched.borrow_mut().push(watch);
                }
            }
        })
        .register();
    // The registry announces every existing global before it answers the first sync, and the
    // nodes bound meanwhile send their info before it answers the second.
    for _ in 0..2 {
        roundtrip(&mainloop, &core)?;
    }
    drop(_registry_listener);
    watched.borrow_mut().clear();
    let nodes = nodes.take().into_values().collect();
    Ok(group_applications(nodes))
}

/// Runs the loop until the server has answered everything sent so far.
fn roundtrip(mainloop: &MainLoopRc, core: &CoreRc) -> Result<(), String> {
    let pending = core.sync(0).map_err(|e| format!("PipeWire sync: {e}"))?;
    let _core_listener = core
        .add_listener_local()
        .done({
            let mainloop = mainloop.clone();
            move |id, seq| {
                if id == pw::core::PW_ID_CORE && seq == pending {
                    mainloop.quit();
                }
            }
        })
        .register();
    mainloop.run();
    Ok(())
}

/// One target per process, or per name for streams without a process id, sorted by name and
/// then process id so the list is stable between refreshes.
fn group_applications(nodes: Vec<NodeInfo>) -> Vec<AudioTarget> {
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    enum Key {
        Pid(u32),
        Name(String),
    }
    let mut groups: BTreeMap<Key, Target> = BTreeMap::new();
    for node in nodes {
        if node.application.is_empty() && node.pid.is_none() {
            continue;
        }
        let key = match node.pid {
            Some(pid) => Key::Pid(pid),
            None => Key::Name(node.application.clone()),
        };
        groups.entry(key).or_insert(Target {
            pid: node.pid,
            application: node.application,
        });
    }
    let mut targets: Vec<Target> = groups.into_values().collect();
    targets.sort_by(|a, b| {
        a.application
            .to_lowercase()
            .cmp(&b.application.to_lowercase())
            .then(a.pid.cmp(&b.pid))
    });
    targets
        .into_iter()
        .map(|target| AudioTarget {
            name: target.application.clone(),
            pid: target.pid,
            settings: target.settings(),
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The graph while capturing

/// What the capture thread knows of the graph: the application streams, the audio ports of
/// every node, its own node once the stream has one, and the links it holds.
struct Graph {
    core: CoreRc,
    registry: RegistryRc,
    target: Target,
    me: Option<u32>,
    nodes: HashMap<u32, NodeInfo>,
    /// The bound proxies of the application nodes, for their info.
    watched: HashMap<u32, (Node, NodeListener)>,
    ports: HashMap<u32, PortInfo>,
    /// Keyed by output port and input port; dropping a link's proxy removes it from the graph.
    links: HashMap<(u32, u32), Link>,
}

impl Graph {
    /// Records a global; true when it is a node or port the capture may care about. An
    /// application node is bound as well, since the registry's announcement lacks its process
    /// id; the info that follows updates the node and relinks.
    fn add(graph: &Rc<RefCell<Graph>>, global: &GlobalObject<&DictRef>) -> bool {
        let mut this = graph.borrow_mut();
        if is_output_stream(global) {
            let id = global.id;
            if let Some(props) = global.props {
                this.nodes.insert(id, node_info(props));
            }
            let watch = watch_node(&this.registry, global, {
                let graph = graph.clone();
                move |info| {
                    let mut this = graph.borrow_mut();
                    if this.nodes.get(&id) != Some(&info) {
                        this.nodes.insert(id, info);
                        this.reconcile();
                    }
                }
            });
            if let Some(watch) = watch {
                this.watched.insert(id, watch);
            }
            return true;
        }
        if let Some(port) = audio_port(global) {
            this.ports.insert(global.id, port);
            return true;
        }
        false
    }

    /// Forgets a global; true when something the capture knew of went away.
    fn remove(&mut self, id: u32) -> bool {
        let mut changed = self.nodes.remove(&id).is_some();
        self.watched.remove(&id);
        changed |= self.ports.remove(&id).is_some();
        if self.me == Some(id) {
            self.me = None;
            changed = true;
        }
        let before = self.links.len();
        self.links
            .retain(|(output, input), _| *output != id && *input != id);
        changed || self.links.len() != before
    }

    /// Brings the links in line with the graph: one from each output port of every target node
    /// to the input port of the same channel, or to every input port when there is none.
    fn reconcile(&mut self) {
        let Some(me) = self.me else {
            return;
        };
        let inputs: Vec<(u32, &str)> = self
            .ports
            .iter()
            .filter(|(_, port)| port.input && port.node == me)
            .map(|(id, port)| (*id, port.channel.as_str()))
            .collect();
        let targets = select_nodes(&self.nodes, &self.target);
        let outputs: Vec<(u32, &str)> = self
            .ports
            .iter()
            .filter(|(_, port)| !port.input && targets.contains(&port.node))
            .map(|(id, port)| (*id, port.channel.as_str()))
            .collect();
        let wanted = plan_links(&outputs, &inputs);
        self.links.retain(|key, _| wanted.contains(key));
        for (output, input) in wanted {
            if self.links.contains_key(&(output, input)) {
                continue;
            }
            let output_node = self.ports[&output].node;
            let link = self.core.create_object::<Link>(
                "link-factory",
                &properties! {
                    "link.output.node" => output_node.to_string(),
                    "link.output.port" => output.to_string(),
                    "link.input.node" => me.to_string(),
                    "link.input.port" => input.to_string(),
                    "object.linger" => "false",
                },
            );
            match link {
                Ok(link) => {
                    self.links.insert((output, input), link);
                }
                Err(error) => eprintln!("aspen-obs-capture: PipeWire link refused: {error}"),
            }
        }
    }
}

/// The nodes the target names: those of its process while any exist, else those of its name.
fn select_nodes(nodes: &HashMap<u32, NodeInfo>, target: &Target) -> Vec<u32> {
    let mut by_pid: Vec<u32> = match target.pid {
        Some(pid) => nodes
            .iter()
            .filter(|(_, node)| node.pid == Some(pid))
            .map(|(id, _)| *id)
            .collect(),
        None => Vec::new(),
    };
    if by_pid.is_empty() && !target.application.is_empty() {
        by_pid = nodes
            .iter()
            .filter(|(_, node)| node.application == target.application)
            .map(|(id, _)| *id)
            .collect();
    }
    by_pid.sort_unstable();
    by_pid
}

/// Pairs output ports with input ports by channel name; an output whose channel has no input
/// of its own (mono, or a surround channel) goes to every input. Sorted, so two plans over
/// the same ports are equal.
fn plan_links(outputs: &[(u32, &str)], inputs: &[(u32, &str)]) -> Vec<(u32, u32)> {
    let mut links = Vec::new();
    for (output, channel) in outputs {
        let same: Vec<u32> = inputs
            .iter()
            .filter(|(_, input_channel)| input_channel == channel)
            .map(|(input, _)| *input)
            .collect();
        if same.is_empty() {
            links.extend(inputs.iter().map(|(input, _)| (*output, *input)));
        } else {
            links.extend(same.into_iter().map(|input| (*output, input)));
        }
    }
    links.sort_unstable();
    links.dedup();
    links
}

// ---------------------------------------------------------------------------------------------
// The capture thread

enum Command {
    Retarget(Target),
    Quit,
}

/// A running capture: the thread running the PipeWire loop and the way to reach it. Dropping
/// it ends the capture and waits for the thread.
pub struct Capture {
    control: pw::channel::Sender<Command>,
    thread: Option<JoinHandle<()>>,
}

impl Capture {
    /// Starts capturing `target`, calling `on_audio` with each buffer of interleaved 48 kHz
    /// stereo float on PipeWire's real-time thread, where it must not block. Fails when
    /// PipeWire cannot be reached.
    pub fn start(
        target: Target,
        on_audio: impl FnMut(&[f32]) + Send + 'static,
    ) -> Result<Capture, String> {
        let (control, commands) = pw::channel::channel();
        let (ready, started) = mpsc::channel();
        let delivery = Delivery {
            on_audio: Box::new(on_audio),
            scratch: Vec::new(),
        };
        let thread = std::thread::Builder::new()
            .name("aspen-pipewire-audio".into())
            .spawn(move || run(delivery, target, commands, ready))
            .map_err(|error| format!("could not start the PipeWire thread: {error}"))?;
        match started.recv_timeout(CONNECT_TIMEOUT) {
            Ok(Ok(())) => Ok(Capture {
                control,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = control.send(Command::Quit);
                Err("PipeWire did not answer".to_string())
            }
        }
    }

    /// Captures `target` instead, relinking as the graph stands.
    #[allow(dead_code)]
    pub fn retarget(&self, target: Target) {
        let _ = self.control.send(Command::Retarget(target));
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.control.send(Command::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Where a capture's samples go, called on PipeWire's real-time thread.
type AudioSink = Box<dyn FnMut(&[f32]) + Send>;

/// What the process callback needs: where the samples go, and a buffer to unpack them into
/// without allocating on the real-time thread.
struct Delivery {
    on_audio: AudioSink,
    scratch: Vec<f32>,
}

fn run(
    delivery: Delivery,
    target: Target,
    commands: pw::channel::Receiver<Command>,
    ready: mpsc::Sender<Result<(), String>>,
) {
    let outcome = (|| -> Result<(), String> {
        pw::init();
        let mainloop = MainLoopRc::new(None).map_err(|e| format!("PipeWire loop: {e}"))?;
        let context =
            ContextRc::new(&mainloop, None).map_err(|e| format!("PipeWire context: {e}"))?;
        let core = context
            .connect_rc(None)
            .map_err(|e| format!("could not connect to PipeWire: {e}"))?;
        let registry = core
            .get_registry_rc()
            .map_err(|e| format!("PipeWire registry: {e}"))?;
        let graph = Rc::new(RefCell::new(Graph {
            core: core.clone(),
            registry: registry.clone(),
            target,
            me: None,
            nodes: HashMap::new(),
            watched: HashMap::new(),
            ports: HashMap::new(),
            links: HashMap::new(),
        }));
        let _registry_listener = registry
            .add_listener_local()
            .global({
                let graph = graph.clone();
                move |global| {
                    if Graph::add(&graph, global) {
                        graph.borrow_mut().reconcile();
                    }
                }
            })
            .global_remove({
                let graph = graph.clone();
                move |id| {
                    let mut graph = graph.borrow_mut();
                    if graph.remove(id) {
                        graph.reconcile();
                    }
                }
            })
            .register();
        let _core_listener = core
            .add_listener_local()
            .error(|id, _seq, res, message| {
                eprintln!("aspen-obs-capture: PipeWire error on {id} ({res}): {message}");
            })
            .register();

        let stream = StreamRc::new(
            core.clone(),
            "Aspen game audio",
            properties! {
                "media.type" => "Audio",
                "media.category" => "Capture",
                "media.role" => "Game",
                "node.name" => "aspen-game-audio",
                "node.description" => "Aspen game audio",
                // The links are made here; the session manager must not add its own.
                "node.autoconnect" => "false",
            },
        )
        .map_err(|e| format!("PipeWire stream: {e}"))?;
        let _stream_listener = stream
            .add_local_listener_with_user_data(delivery)
            .state_changed({
                let graph = graph.clone();
                move |stream, _, _, state| match state {
                    // The node exists from Paused on; its ports follow through the registry.
                    StreamState::Paused | StreamState::Streaming => {
                        let id = stream.node_id();
                        let mut graph = graph.borrow_mut();
                        if graph.me != Some(id) {
                            graph.me = Some(id);
                            graph.reconcile();
                        }
                    }
                    StreamState::Error(error) => {
                        eprintln!("aspen-obs-capture: PipeWire stream failed: {error}");
                    }
                    _ => {}
                }
            })
            .process(|stream, delivery| {
                if let Some(mut buffer) = stream.dequeue_buffer() {
                    deliver(delivery, &mut buffer);
                }
            })
            .register()
            .map_err(|e| format!("PipeWire stream listener: {e}"))?;
        let format = format_param();
        let mut params = [spa::pod::Pod::from_bytes(&format).ok_or("audio format pod")?];
        stream
            .connect(
                spa::utils::Direction::Input,
                None,
                StreamFlags::MAP_BUFFERS | StreamFlags::RT_PROCESS,
                &mut params,
            )
            .map_err(|e| format!("PipeWire stream connect: {e}"))?;

        let _commands = commands.attach(mainloop.loop_(), {
            let graph = graph.clone();
            let mainloop = mainloop.clone();
            move |command| match command {
                Command::Quit => mainloop.quit(),
                Command::Retarget(target) => {
                    let mut graph = graph.borrow_mut();
                    if graph.target != target {
                        graph.target = target;
                        graph.reconcile();
                    }
                }
            }
        });
        let _ = ready.send(Ok(()));
        mainloop.run();
        {
            let mut graph = graph.borrow_mut();
            graph.links.clear();
            graph.watched.clear();
        }
        let _ = stream.disconnect();
        Ok(())
    })();
    if let Err(error) = outcome {
        // Unheard when the loop already ran and `start` returned; the log then stands.
        eprintln!("aspen-obs-capture: PipeWire capture ended: {error}");
        let _ = ready.send(Err(error));
    }
}

/// The format asked of the stream: interleaved float, 48 kHz, front left and right. The
/// adapter converts whatever the linked ports carry.
fn format_param() -> Vec<u8> {
    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::F32LE);
    info.set_rate(RATE);
    info.set_channels(CHANNELS);
    let mut position = [0; spa::sys::SPA_AUDIO_MAX_CHANNELS as usize];
    position[0] = spa::sys::SPA_AUDIO_CHANNEL_FL;
    position[1] = spa::sys::SPA_AUDIO_CHANNEL_FR;
    info.set_position(position);
    let object = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: info.into(),
    };
    spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(object),
    )
    .expect("audio format serialises")
    .0
    .into_inner()
}

/// Hands one buffer of the stream to the capture's callback as interleaved float. Runs on
/// PipeWire's real-time thread.
fn deliver(delivery: &mut Delivery, buffer: &mut pw::buffer::Buffer<'_>) {
    let datas = buffer.datas_mut();
    let Some(data) = datas.first_mut() else {
        return;
    };
    let offset = data.chunk().offset() as usize;
    let size = data.chunk().size() as usize;
    let Some(bytes) = data.data() else {
        return;
    };
    let end = offset.saturating_add(size).min(bytes.len());
    let start = offset.min(end);
    let samples = &bytes[start..end];
    let frame_bytes = std::mem::size_of::<f32>() * CHANNELS as usize;
    let whole = samples.len() / frame_bytes * frame_bytes;
    if whole == 0 {
        return;
    }
    delivery.scratch.clear();
    delivery.scratch.extend(
        samples[..whole]
            .as_chunks::<{ std::mem::size_of::<f32>() }>()
            .0
            .iter()
            .map(|bytes| f32::from_le_bytes(*bytes)),
    );
    (delivery.on_audio)(&delivery.scratch);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(application: &str, pid: Option<u32>) -> NodeInfo {
        NodeInfo {
            application: application.into(),
            pid,
        }
    }

    #[test]
    fn applications_are_grouped_by_process_and_sorted_by_name() {
        let targets = group_applications(vec![
            node("Firefox", Some(10)),
            node("Firefox", Some(10)),
            node("doom.exe", Some(20)),
            node("Chromium", Some(30)),
            node("Chromium", Some(31)),
            node("", None),
            node("Nameless process", None),
        ]);
        let summary: Vec<(String, Option<u32>)> = targets
            .iter()
            .map(|target| (target.name.clone(), target.pid))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("Chromium".into(), Some(30)),
                ("Chromium".into(), Some(31)),
                ("doom.exe".into(), Some(20)),
                ("Firefox".into(), Some(10)),
                ("Nameless process".into(), None),
            ]
        );
        assert_eq!(
            targets[0].settings,
            serde_json::json!({ "application": "Chromium", "pid": 30 })
        );
        assert_eq!(
            targets[4].settings,
            serde_json::json!({ "application": "Nameless process" })
        );
    }

    #[test]
    fn the_process_is_the_target_while_it_plays_and_its_name_after_a_restart() {
        let mut nodes = HashMap::new();
        nodes.insert(1, node("doom.exe", Some(20)));
        nodes.insert(2, node("doom.exe", Some(20)));
        nodes.insert(3, node("doom.exe", Some(99)));
        nodes.insert(4, node("Firefox", Some(10)));
        let target = Target {
            pid: Some(20),
            application: "doom.exe".into(),
        };
        assert_eq!(select_nodes(&nodes, &target), vec![1, 2]);
        nodes.remove(&1);
        nodes.remove(&2);
        assert_eq!(select_nodes(&nodes, &target), vec![3]);
        nodes.remove(&3);
        assert!(select_nodes(&nodes, &target).is_empty());
        let by_name = Target {
            pid: None,
            application: "Firefox".into(),
        };
        assert_eq!(select_nodes(&nodes, &by_name), vec![4]);
        assert!(select_nodes(&nodes, &Target::default()).is_empty());
    }

    #[test]
    fn links_pair_channels_by_name_and_fold_the_rest_into_both() {
        let inputs = [(100, "FL"), (101, "FR")];
        assert_eq!(
            plan_links(&[(1, "FL"), (2, "FR")], &inputs),
            vec![(1, 100), (2, 101)]
        );
        assert_eq!(
            plan_links(&[(5, "MONO")], &inputs),
            vec![(5, 100), (5, 101)]
        );
        assert_eq!(
            plan_links(&[(1, "FL"), (2, "FR"), (3, "FC"), (4, "LFE")], &inputs),
            vec![(1, 100), (2, 101), (3, 100), (3, 101), (4, 100), (4, 101)]
        );
        assert!(plan_links(&[(1, "FL")], &[]).is_empty());
    }

    #[test]
    fn the_target_is_written_the_way_the_capture_reads_it() {
        let target = Target {
            pid: Some(7),
            application: "game".into(),
        };
        let settings = target.settings();
        assert_eq!(
            settings,
            serde_json::json!({ "application": "game", "pid": 7 })
        );
        assert_eq!(
            Target::from_settings(Some(&settings.to_string())).unwrap(),
            target
        );
        assert_eq!(
            Target::from_settings(Some(r#"{"application":"game","pid":0}"#)).unwrap(),
            Target {
                pid: None,
                application: "game".into()
            }
        );
        assert_eq!(Target::from_settings(None).unwrap(), Target::default());
        assert!(Target::from_settings(Some("7")).is_err());
    }
}
