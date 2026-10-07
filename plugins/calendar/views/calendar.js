// An event calendar: its events soonest first, each with how many are going and a way to go or
// not, and for its creator a way to delete it, a form to add one, and the person's own feed
// address for their calendar app. Everything goes through the bridge (`bridge.js`), which calls
// the plugin's routes as the person.
"use strict";

const calendar = document.getElementById("calendar");

function element(tag, attributes = {}, ...children) {
  const node = document.createElement(tag);
  for (const [name, value] of Object.entries(attributes)) {
    if (name === "onclick") node.addEventListener("click", value);
    else if (name === "onsubmit") node.addEventListener("submit", value);
    else node.setAttribute(name, value);
  }
  for (const child of children) node.append(child);
  return node;
}

function when(ms) {
  return new Date(ms).toLocaleString(aspen.context.locale, {
    dateStyle: "full",
    timeStyle: "short",
  });
}

/** Why something did not work: `forbidden` when it was refused, and a full calendar. */
function failed(error, forbidden = "cannotAdd") {
  const key = error.status === 403 ? forbidden : error.status === 409 ? "full" : "failed";
  return element("p", { class: "error", role: "alert" }, aspen.t(key));
}

async function show() {
  calendar.setAttribute("aria-busy", "true");
  const channel = aspen.context.channel;
  const me = aspen.context.user.id;
  let events;
  try {
    events = await aspen.json("GET", `calendars/${channel}/events`);
  } catch (error) {
    calendar.replaceChildren(failed(error));
    return;
  }
  const title = element("input", { name: "title", required: "", maxlength: "200" });
  const start = element("input", { name: "start", type: "datetime-local", required: "" });
  const form = element(
    "form",
    {
      onsubmit: async (event) => {
        event.preventDefault();
        try {
          // The input is in the reader's own time zone; the plugin keeps the moment.
          const at = new Date(start.value).toISOString();
          await aspen.json("POST", `calendars/${channel}/events`, { title: title.value, start: at });
          await show();
        } catch (error) {
          form.append(failed(error));
        }
      },
    },
    element("h2", {}, aspen.t("newEvent")),
    element("label", {}, aspen.t("title"), title),
    element("label", {}, aspen.t("start"), start),
    element("button", { type: "submit", class: "primary" }, aspen.t("add")),
  );
  const list = element("ul", {});
  for (const event of events) {
    const going = event.youAreGoing;
    const item = element(
      "li",
      { class: "post" },
      element("h2", {}, event.title),
      element(
        "div",
        { class: "meta" },
        `${when(event.start)} · ${aspen.t("goingCount", { count: event.goingCount })}`,
      ),
      element(
        "button",
        {
          type: "button",
          class: going ? "" : "primary",
          "aria-pressed": String(going),
          onclick: async () => {
            await aspen.request("POST", `calendars/${channel}/events/${event.id}/rsvp`);
            await show();
          },
        },
        going ? aspen.t("notGoingButton") : aspen.t("goingButton"),
      ),
    );
    if (event.creator === me) {
      item.append(
        " ",
        element(
          "button",
          {
            type: "button",
            onclick: async () => {
              const answer = await aspen.request(
                "DELETE",
                `calendars/${channel}/events/${event.id}`,
              );
              if (answer.status >= 400) item.append(failed(answer, "cannotDelete"));
              else await show();
            },
          },
          aspen.t("delete"),
        ),
      );
    }
    list.append(item);
  }
  const feed = element("div", {});
  const subscribe = element(
    "button",
    {
      type: "button",
      onclick: async () => {
        try {
          const { path } = await aspen.json("POST", `calendars/${channel}/feed`);
          const address = element("input", {
            readonly: "",
            value: new URL(path, aspen.context.apiBase).href,
            "aria-label": aspen.t("subscribe"),
          });
          feed.replaceChildren(address, element("p", { class: "hint" }, aspen.t("subscribeHint")));
          address.select();
        } catch (error) {
          feed.replaceChildren(failed(error));
        }
      },
    },
    aspen.t("subscribe"),
  );
  calendar.replaceChildren(
    events.length === 0 ? element("p", { class: "hint" }, aspen.t("empty")) : list,
    form,
    subscribe,
    feed,
  );
  calendar.setAttribute("aria-busy", "false");
}

aspen.onHello(() => {
  void show();
});

aspen.onEvent((event) => {
  if (event.kind === "changed") void show();
});
