// A blackjack table: the dealer, the seats, and what the person may do now. Everything goes
// through the bridge (`bridge.js`), which calls the plugin's routes as the person; the plugin's
// `table` events carry the table as everyone at it sees it.
"use strict";

const root = document.getElementById("table");
const names = new Map();
const RANKS = ["A", "2", "3", "4", "5", "6", "7", "8", "9", "10", "J", "Q", "K"];
const SUITS = ["♠", "♥", "♦", "♣"];
const STEP = 10;
const MIN_BET = 10;
const MAX_BET = 500;

let table = null;
let chips = null;
let amount = 50;
let error = null;
let busy = false;

function element(tag, attributes = {}, ...children) {
  const node = document.createElement(tag);
  for (const [name, value] of Object.entries(attributes)) {
    if (name === "onclick") node.addEventListener("click", value);
    else if (name === "oninput") node.addEventListener("input", value);
    else if (value === false || value == null) continue;
    else node.setAttribute(name, value === true ? "" : value);
  }
  for (const child of children) if (child != null) node.append(child);
  return node;
}

function number(n) {
  return new Intl.NumberFormat(aspen.context.locale).format(n);
}

/** What cards count for together, and whether an ace counts as 11. */
function total(cards) {
  let hard = 0;
  let ace = false;
  for (const card of cards) {
    const rank = (card % 13) + 1;
    hard += Math.min(rank, 10);
    ace ||= rank === 1;
  }
  return ace && hard <= 11 ? { total: hard + 10, soft: true } : { total: hard, soft: false };
}

function cardView(card) {
  if (card === null) {
    return element("div", { class: "card back", role: "img", "aria-label": aspen.t("hidden") });
  }
  const rank = card % 13;
  const suit = Math.floor(card / 13);
  const label = aspen.t("cardName", {
    rank: aspen.t(`rank${rank + 1}`),
    suit: aspen.t(`suit${suit}`),
  });
  return element(
    "div",
    { class: suit === 1 || suit === 2 ? "card red" : "card", role: "img", "aria-label": label },
    element("span", { "aria-hidden": "true" }, RANKS[rank]),
    element("span", { "aria-hidden": "true" }, SUITS[suit]),
  );
}

function totalText(cards) {
  const { total: value, soft } = total(cards);
  return soft && value < 21
    ? aspen.t("softTotal", { total: value })
    : aspen.t("total", { total: value });
}

function outcomeView(hand) {
  switch (hand.outcome) {
    case "blackjack":
      return element(
        "span",
        { class: "outcome good" },
        aspen.t("blackjack", { amount: number((hand.stake * 3) / 2) }),
      );
    case "win":
      return element(
        "span",
        { class: "outcome good" },
        aspen.t("win", { amount: number(hand.stake) }),
      );
    case "push":
      return element("span", { class: "outcome" }, aspen.t("push", { amount: number(hand.stake) }));
    case "lose":
      return element("span", { class: "outcome bad" }, aspen.t("lose"));
    case "bust":
      return element("span", { class: "outcome bad" }, aspen.t("bust"));
    default:
      return total(hand.cards).total > 21
        ? element("span", { class: "outcome bad" }, aspen.t("bust"))
        : null;
  }
}

function secondsLeft(deadline) {
  return Math.max(0, Math.ceil((deadline - Date.now()) / 1000));
}

function nameOf(user) {
  if (user === aspen.context.user.id) return aspen.t("you");
  return names.get(user) ?? aspen.t("someone");
}

function mySeat() {
  return table?.seats.findIndex((seat) => seat.user === aspen.context.user.id) ?? -1;
}

/** The status line: whose turn it is, how long is left, or what happens next. */
function status() {
  const phase = table.phase;
  switch (phase.name) {
    case "betting":
      return phase.deadline == null
        ? aspen.t("waitingForBets")
        : aspen.t("waitingForDeal", { seconds: secondsLeft(phase.deadline) });
    case "playing": {
      const seconds = secondsLeft(phase.deadline);
      const user = table.seats[phase.seat].user;
      return user === aspen.context.user.id
        ? aspen.t("yourTurn", { seconds })
        : aspen.t("turnOf", { name: nameOf(user), seconds });
    }
    case "dealer":
      return aspen.t("dealerPlays");
    default:
      return aspen.t("nextRound");
  }
}

async function call(method, path, body) {
  if (busy) return;
  busy = true;
  error = null;
  try {
    const answer = await aspen.json(method, `tables/${aspen.context.channel}${path}`, body);
    accept(answer.table);
    chips = answer.chips;
  } catch (failure) {
    let key = "failed";
    try {
      key = JSON.parse(failure.message).error ?? key;
    } catch {
      // Not the plugin's answer: the deployment could not be reached.
    }
    error = aspen.t(key);
  } finally {
    busy = false;
    render();
  }
}

/** Takes a table unless a later one is already shown. */
function accept(next) {
  if (table === null || next.version >= table.version || next.round > table.round) table = next;
}

function controls() {
  const phase = table.phase;
  const seat = mySeat();
  const box = element("div", { class: "controls" });
  const open = phase.name === "settled" || (phase.name === "betting" && seat === -1);
  if (open) {
    const input = element("input", {
      type: "number",
      min: MIN_BET,
      max: MAX_BET,
      step: STEP,
      value: amount,
      "aria-label": aspen.t("betAmount"),
      "data-focus": "amount",
      oninput: (event) => {
        amount = Number(event.target.value);
        betButton.textContent = aspen.t("placeBet", { amount: number(amount) });
      },
    });
    const betButton = element(
      "button",
      {
        type: "button",
        class: "primary",
        disabled: busy,
        "data-focus": "bet",
        onclick: () => call("POST", "/bet", { amount }),
      },
      aspen.t("placeBet", { amount: number(amount) }),
    );
    box.append(input);
    for (const preset of [10, 50, 100, 500]) {
      box.append(
        element(
          "button",
          {
            type: "button",
            "aria-pressed": String(amount === preset),
            "data-focus": `preset${preset}`,
            onclick: () => {
              amount = preset;
              render();
            },
          },
          number(preset),
        ),
      );
    }
    box.append(betButton);
  } else if (phase.name === "betting" && seat !== -1) {
    const mine = table.seats[seat];
    box.append(
      element(
        "button",
        {
          type: "button",
          class: "primary",
          disabled: busy || mine.ready || !mine.confirmed,
          "data-focus": "deal",
          onclick: () => call("POST", "/ready"),
        },
        mine.ready ? aspen.t("ready") : aspen.t("deal"),
      ),
      element(
        "button",
        {
          type: "button",
          disabled: busy || !mine.confirmed,
          "data-focus": "leave",
          onclick: () => call("DELETE", "/bet"),
        },
        aspen.t("leave"),
      ),
    );
  } else if (phase.name === "playing" && table.seats[phase.seat].user === aspen.context.user.id) {
    const seatNow = table.seats[phase.seat];
    const hand = seatNow.hands[phase.hand];
    const waiting = busy || hand.waiting != null;
    const decide = (action) => () => call("POST", "/actions", { action, version: table.version });
    const canDouble = hand.cards.length === 2 && !hand.doubled && (chips ?? 0) >= hand.stake;
    const canSplit =
      seatNow.hands.length === 1 &&
      hand.cards.length === 2 &&
      Math.min((hand.cards[0] % 13) + 1, 10) === Math.min((hand.cards[1] % 13) + 1, 10) &&
      (chips ?? 0) >= hand.stake;
    box.append(
      element(
        "button",
        {
          type: "button",
          class: "primary",
          disabled: waiting,
          "data-focus": "hit",
          onclick: decide("hit"),
        },
        aspen.t("hit"),
      ),
      element(
        "button",
        {
          type: "button",
          class: "primary",
          disabled: waiting,
          "data-focus": "stand",
          onclick: decide("stand"),
        },
        aspen.t("stand"),
      ),
    );
    if (canDouble) {
      box.append(
        element(
          "button",
          { type: "button", disabled: waiting, "data-focus": "double", onclick: decide("double") },
          aspen.t("double"),
        ),
      );
    }
    if (canSplit) {
      box.append(
        element(
          "button",
          { type: "button", disabled: waiting, "data-focus": "split", onclick: decide("split") },
          aspen.t("split"),
        ),
      );
    }
  }
  return box;
}

function seatView(seat, index) {
  const phase = table.phase;
  const mine = seat.user === aspen.context.user.id;
  const hands = seat.hands.map((hand, h) => {
    const active = phase.name === "playing" && phase.seat === index && phase.hand === h;
    return element(
      "div",
      { class: active ? "hand active" : "hand", "aria-current": active ? "true" : null },
      element("div", { class: "cards" }, ...hand.cards.map(cardView)),
      element(
        "div",
        { class: "meta" },
        `${totalText(hand.cards)} · ${aspen.t("stake", { amount: number(hand.stake) })}`,
      ),
      outcomeView(hand),
    );
  });
  return element(
    "li",
    { class: mine ? "seat mine" : "seat" },
    element("div", { class: "name" }, nameOf(seat.user)),
    element(
      "div",
      { class: "meta" },
      `${aspen.t("bet")}: ${number(seat.bet)}${seat.ready && phase.name === "betting" ? ` · ${aspen.t("ready")}` : ""}`,
    ),
    ...hands,
  );
}

const board = element("div", { class: "board" });
const statusLine = element("p", { class: "meta" });
// What a screen reader hears: the status when it says something new, not each second's tick.
const announcer = element("p", { class: "sr-only", "aria-live": "polite" });
root.replaceChildren(board, statusLine, announcer);
let announced = "";

/** Shows the status line, telling a screen reader only of what changed besides the seconds. */
function showStatus() {
  const line = status();
  statusLine.textContent = line;
  const words = line.replace(/\d+/g, "");
  if (words !== announced) {
    announced = words;
    announcer.textContent = line;
  }
}

function render() {
  if (table === null) return;
  // Keyboard focus survives the table being drawn again.
  const focused = document.activeElement?.dataset?.focus;
  const dealer = element(
    "section",
    { class: "dealer", "aria-label": aspen.t("dealer") },
    element("h2", {}, aspen.t("dealer")),
    element("div", { class: "cards" }, ...table.dealer.map(cardView)),
    table.dealerTotal == null
      ? null
      : element("div", { class: "meta" }, aspen.t("total", { total: table.dealerTotal })),
  );
  const seats = element("ul", { class: "seats" }, ...table.seats.map(seatView));
  board.replaceChildren(
    element(
      "div",
      { class: "bar" },
      element(
        "span",
        { class: "chips" },
        `${aspen.t("chipsLabel")}: ${chips == null ? "…" : aspen.t("chips", { count: number(chips) })}`,
      ),
      element("span", { class: "hint" }, aspen.t("notMoney")),
    ),
    element("div", { class: "felt" }, dealer, seats),
    controls(),
    ...(error == null ? [] : [element("p", { class: "error", role: "alert" }, error)]),
  );
  showStatus();
  if (focused) board.querySelector(`[data-focus="${focused}"]`)?.focus();
  root.setAttribute("aria-busy", "false");
}

/** Learns the names of everyone seated, then shows the table again. */
async function learnNames() {
  const unknown = table.seats.map((seat) => seat.user).filter((id) => !names.has(id));
  if (unknown.length === 0) return;
  for (const user of await aspen.users(unknown)) names.set(user.id, user.displayName ?? user.name);
  render();
}

async function load() {
  try {
    const answer = await aspen.json("GET", `tables/${aspen.context.channel}`);
    table = answer.table;
    chips = answer.chips;
    error = null;
  } catch {
    error = aspen.t("failed");
    board.replaceChildren(element("p", { class: "error", role: "alert" }, error));
    return;
  }
  render();
  void learnNames();
}

aspen.onHello(() => {
  void load();
});
aspen.onTheme(() => render());

aspen.onEvent((event) => {
  if (event.kind !== "table" || table === null) return;
  const before = table.phase.name;
  accept(event.payload);
  render();
  void learnNames();
  // The timer pays a round's winners before it says the round is settled.
  if (table.phase.name === "settled" && before !== "settled" && mySeat() !== -1) void load();
});

// The countdowns.
setInterval(() => {
  if (table !== null && table.phase.deadline != null) showStatus();
}, 1000);
