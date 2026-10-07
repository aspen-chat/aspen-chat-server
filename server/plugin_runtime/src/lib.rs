//! The WebAssembly side of plugins (`app::plugin::host`): the bindings `spec/plugin.wit`
//! generates, the engine every plugin runs on and the epoch that keeps its calls' deadlines, and
//! what one call's store may take. What a plugin may do through the host is the server's.

use aspen_wire::plugin::PluginText;
use std::sync::OnceLock;
use std::time::Duration;
use wasmtime::Engine;

/// The bindings `spec/plugin.wit` generates.
mod bindings {
    wasmtime::component::bindgen!({
        path: "../../spec/plugin.wit",
        world: "plugin",
        imports: { default: async },
        exports: { default: async },
    });
}

pub use bindings::aspen::plugin::types as wit;
pub use bindings::{Plugin, PluginPre, aspen};

/// How often the engine's epoch ticks, which is how finely deadlines are kept.
const TICK: Duration = Duration::from_millis(1);

/// The most core and component instances one call's component may make. The examples, built
/// for `wasm32-wasip2`, make three.
const MAX_INSTANCES: usize = 16;

/// The most tables one call may hold; the examples hold two.
const MAX_TABLES: usize = 8;

/// The most memories one call may hold, whose sizes together are what `[plugins] memory_mib`
/// bounds; the examples hold one.
const MAX_MEMORIES: usize = 4;

/// The most elements one table may grow to.
const MAX_TABLE_ELEMENTS: usize = 50_000;

/// What one call may take: its memories' bytes together at most `[plugins] memory_mib`, and a
/// bounded number of instances, tables, and table elements.
pub struct CallLimits {
    /// The bytes its memories may still grow by.
    pub memory_left: usize,
}

impl wasmtime::ResourceLimiter for CallLimits {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let more = desired.saturating_sub(current);
        if more > self.memory_left {
            return Ok(false);
        }
        // Counted before the memory grows; a growth that then fails stays counted, which only
        // errs toward less.
        self.memory_left -= more;
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= MAX_TABLE_ELEMENTS)
    }

    fn instances(&self) -> usize {
        MAX_INSTANCES
    }

    fn tables(&self) -> usize {
        MAX_TABLES
    }

    fn memories(&self) -> usize {
        MAX_MEMORIES
    }
}

/// The engine every plugin of this server runs on.
pub fn engine() -> wasmtime::Result<Engine> {
    let mut config = wasmtime::Config::new();
    config.epoch_interruption(true);
    config.wasm_component_model(true);
    Engine::new(&config)
}

/// Ticks the engine's epoch for as long as the process runs. Started once.
pub fn start_ticker(engine: Engine) {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        std::thread::Builder::new()
            .name("plugin-epoch".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(TICK);
                    engine.increment_epoch();
                }
            })
            .expect("the plugin epoch thread starts");
    });
}

impl From<&PluginText> for wit::Text {
    fn from(text: &PluginText) -> Self {
        wit::Text {
            key: text.key.clone(),
            args: text
                .args
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        }
    }
}

impl From<wit::Text> for PluginText {
    fn from(text: wit::Text) -> Self {
        PluginText {
            key: text.key,
            args: text.args.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime::component::{Component, Linker};
    use wasmtime::{ResourceLimiter, Store};

    #[test]
    fn a_calls_memories_share_one_ceiling() {
        let mut limits = CallLimits { memory_left: 100 };
        assert!(limits.memory_growing(0, 60, None).unwrap());
        // A second memory draws on what the first left.
        assert!(!limits.memory_growing(0, 60, None).unwrap());
        assert!(limits.memory_growing(0, 40, None).unwrap());
        assert!(!limits.memory_growing(40, 41, None).unwrap());
        assert!(
            !limits
                .table_growing(0, MAX_TABLE_ELEMENTS + 1, None)
                .unwrap()
        );
    }

    /// The example plugins, once built for `wasm32-wasip2` (as CI builds them), instantiate
    /// within a call's limits; a plugin not built is passed over.
    #[tokio::test]
    async fn the_example_plugins_fit_a_calls_limits() {
        let engine = engine().unwrap();
        for name in ["word_filter", "forum", "calendar"] {
            let path = format!(
                "{}/../../plugins/{name}/target/wasm32-wasip2/release/aspen_{name}.wasm",
                env!("CARGO_MANIFEST_DIR")
            );
            let Ok(bytes) = std::fs::read(&path) else {
                eprintln!("{name} is not built; passing it over");
                continue;
            };
            let component = Component::new(&engine, &bytes).unwrap();
            let mut linker: Linker<CallLimits> = Linker::new(&engine);
            linker.define_unknown_imports_as_traps(&component).unwrap();
            let mut store = Store::new(
                &engine,
                CallLimits {
                    memory_left: 64 << 20,
                },
            );
            store.limiter(|limits| limits);
            store.set_epoch_deadline(u64::MAX / 2);
            linker
                .instantiate_async(&mut store, &component)
                .await
                .unwrap_or_else(|e| panic!("{name} does not fit a call's limits: {e:#}"));
        }
    }
}
