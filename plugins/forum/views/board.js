// A forum board: its posts newest first, a form to post, and a post with its replies. Everything
// goes through the bridge (`bridge.js`), which calls the plugin's routes as the person.
"use strict";

const board = document.getElementById("board");
const names = new Map();
let shown = null; // The post open, or null for the list.

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

/** Learns the names of `ids` from the app. */
async function name(ids) {
  const unknown = ids.filter((id) => !names.has(id));
  if (unknown.length > 0) {
    for (const user of await aspen.users(unknown)) {
      names.set(user.id, user.displayName ?? user.name);
    }
  }
}

function when(ms) {
  return new Date(ms).toLocaleString(aspen.context.locale, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

function failed(error) {
  const message = error.status === 403 ? aspen.t("cannotPost") : aspen.t("failed");
  return element("p", { class: "error", role: "alert" }, message);
}

async function showList() {
  shown = null;
  board.setAttribute("aria-busy", "true");
  const channel = aspen.context.channel;
  let posts;
  try {
    posts = await aspen.json("GET", `boards/${channel}/posts`);
  } catch (error) {
    board.replaceChildren(failed(error));
    return;
  }
  await name(posts.map((p) => p.author));
  const title = element("input", { name: "title", required: "", maxlength: "200" });
  const body = element("textarea", { name: "body" });
  const form = element(
    "form",
    {
      onsubmit: async (event) => {
        event.preventDefault();
        try {
          await aspen.json("POST", `boards/${channel}/posts`, {
            title: title.value,
            body: body.value,
          });
          await showList();
        } catch (error) {
          form.append(failed(error));
        }
      },
    },
    element("h2", {}, aspen.t("newPost")),
    element("label", {}, aspen.t("title"), title),
    element("label", {}, aspen.t("body"), body),
    element("button", { type: "submit", class: "primary" }, aspen.t("post")),
  );
  const list = element("ul", { "aria-label": aspen.context.channelName ?? "" });
  for (const post of posts) {
    list.append(
      element(
        "li",
        { class: "post" },
        element(
          "button",
          { class: "title", type: "button", onclick: () => showPost(post.id) },
          post.title,
        ),
        element(
          "div",
          { class: "meta" },
          `${names.get(post.author) ?? aspen.t("someone")} · ${when(post.createdAt)} · ${
            post.replies === 0 ? aspen.t("noReplies") : aspen.t("replies", { count: post.replies })
          }`,
        ),
      ),
    );
  }
  board.replaceChildren(
    form,
    posts.length === 0 ? element("p", { class: "hint" }, aspen.t("empty")) : list,
  );
  board.setAttribute("aria-busy", "false");
}

async function showPost(id) {
  shown = id;
  board.setAttribute("aria-busy", "true");
  const channel = aspen.context.channel;
  let found;
  try {
    found = await aspen.json("GET", `boards/${channel}/posts/${id}`);
  } catch (error) {
    board.replaceChildren(failed(error));
    return;
  }
  const { post, replies } = found;
  await name([post.author, ...replies.map((r) => r.author)]);
  const body = element("textarea", { name: "reply", required: "" });
  const form = element(
    "form",
    {
      onsubmit: async (event) => {
        event.preventDefault();
        try {
          await aspen.json("POST", `boards/${channel}/posts/${id}/replies`, { body: body.value });
          await showPost(id);
        } catch (error) {
          form.append(failed(error));
        }
      },
    },
    element("label", {}, aspen.t("reply"), body),
    element("button", { type: "submit", class: "primary" }, aspen.t("reply")),
  );
  const thread = element("ul", {});
  for (const reply of replies) {
    thread.append(
      element(
        "li",
        { class: "post" },
        element(
          "div",
          { class: "meta" },
          `${names.get(reply.author) ?? aspen.t("someone")} · ${when(reply.createdAt)}`,
        ),
        element("div", { class: "body" }, reply.body),
      ),
    );
  }
  const actions = element(
    "div",
    {},
    element("button", { type: "button", onclick: () => showList() }, aspen.t("back")),
  );
  if (post.author === aspen.context.user.id) {
    actions.append(
      " ",
      element(
        "button",
        {
          type: "button",
          onclick: async () => {
            await aspen.request("DELETE", `boards/${channel}/posts/${id}`);
            await showList();
          },
        },
        aspen.t("delete"),
      ),
    );
  }
  board.replaceChildren(
    actions,
    element(
      "article",
      { class: "post" },
      element("h1", {}, post.title),
      element(
        "div",
        { class: "meta" },
        `${names.get(post.author) ?? aspen.t("someone")} · ${when(post.createdAt)}`,
      ),
      element("div", { class: "body" }, post.body),
    ),
    thread,
    form,
  );
  board.setAttribute("aria-busy", "false");
}

aspen.onHello(() => {
  void showList();
});

// Someone else posted or replied: show it.
aspen.onEvent((event) => {
  if (event.kind !== "changed") return;
  void (shown === null ? showList() : showPost(shown));
});
