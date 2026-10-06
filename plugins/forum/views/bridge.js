// The view's side of Aspen's plugin bridge (`spec/plugins.md`, Views): it hears `hello` and
// `theme` from the app, laying its colours and fonts over the page, asks the app to call the
// plugin's routes and to name people, and hears the plugin's events. The page has an opaque
// origin and no session; the app is its only way out.
"use strict";

const aspen = (() => {
  const pending = new Map();
  const listeners = { hello: [], theme: [], event: [] };
  let next = 0;
  let context = null;

  function send(message) {
    window.parent.postMessage({ aspen: 1, ...message }, "*");
  }

  /** The person's own faces this page registered, replaced whenever the theme changes. */
  let faces = [];

  /**
   * Gives the page the app's fonts: the stylesheet naming every face the app bundles, and the
   * person's own faces, which come as files, since the page can reach neither by itself.
   */
  function applyFonts(fonts) {
    if (fonts.stylesheet) {
      let link = document.getElementById("aspen-fonts");
      if (link === null) {
        link = document.createElement("link");
        link.id = "aspen-fonts";
        link.rel = "stylesheet";
        document.head.prepend(link);
      }
      if (link.href !== fonts.stylesheet) link.href = fonts.stylesheet;
    }
    for (const face of faces) document.fonts.delete(face);
    faces = [];
    for (const file of fonts.faces ?? []) {
      try {
        const face = new FontFace(file.family, file.data, {
          weight: file.weight,
          style: file.style,
        });
        document.fonts.add(face);
        faces.push(face);
      } catch {
        // A file the browser cannot draw; the stack's next family draws instead.
      }
    }
  }

  /** Lays the app's theme over the page as CSS custom properties, `--aspen-<token>`. */
  function applyTheme(theme) {
    const root = document.documentElement;
    for (const [token, value] of Object.entries(theme.colors)) {
      root.style.setProperty(`--aspen-${token}`, value);
    }
    root.style.setProperty("--aspen-font-sans", theme.fonts.sans);
    root.style.setProperty("--aspen-font-mono", theme.fonts.mono);
    root.style.colorScheme = theme.scheme;
    applyFonts(theme.fonts);
  }

  window.addEventListener("message", (event) => {
    if (event.source !== window.parent) return;
    const message = event.data;
    if (message === null || typeof message !== "object" || message.aspen !== 1) return;
    switch (message.type) {
      case "hello":
        context = message.context;
        document.documentElement.lang = context.locale;
        document.documentElement.dir = context.dir;
        applyTheme(context.theme);
        for (const listener of listeners.hello) listener(context);
        break;
      case "theme":
        if (context !== null) context.theme = message.theme;
        applyTheme(message.theme);
        for (const listener of listeners.theme) listener(message.theme);
        break;
      case "response":
      case "users": {
        const resolve = pending.get(message.id);
        pending.delete(message.id);
        resolve?.(message);
        break;
      }
      case "event":
        for (const listener of listeners.event) listener(message);
        break;
    }
  });

  function ask(message) {
    return new Promise((resolve) => {
      next += 1;
      pending.set(next, resolve);
      send({ ...message, id: next });
    });
  }

  return {
    /** Calls `fn` with the context once the app has said hello. */
    onHello(fn) {
      listeners.hello.push(fn);
      if (context !== null) fn(context);
    },
    onTheme(fn) {
      listeners.theme.push(fn);
    },
    /** Calls `fn` with each of the plugin's events for what this view shows. */
    onEvent(fn) {
      listeners.event.push(fn);
    },
    get context() {
      return context;
    },
    /** The plugin's text of `key`, with `%{name}` filled from `args`. */
    t(key, args = {}) {
      const template = context?.messages?.[key] ?? key;
      return template.replace(/%\{([^}]+)\}/g, (whole, name) => args[name] ?? whole);
    },
    /** Calls the plugin's route as the person, answering `{status, contentType, body}`. */
    async request(method, path, body) {
      const answer = await ask({
        type: "request",
        method,
        path,
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      });
      return answer;
    },
    /** Calls a route and reads its JSON, or throws with the status. */
    async json(method, path, body) {
      const answer = await this.request(method, path, body);
      if (answer.status >= 400)
        throw Object.assign(new Error(answer.body), { status: answer.status });
      return answer.body === "" ? null : JSON.parse(answer.body);
    },
    /** People by id: `{id, name, displayName}` each; one the app cannot find is left out. */
    async users(ids) {
      const answer = await ask({ type: "users", ids });
      return answer.users;
    },
    /** Asks the app to open a channel or message the person may open. */
    open(target) {
      send({ type: "open", ...target });
    },
  };
})();

window.parent.postMessage({ aspen: 1, type: "ready" }, "*");
