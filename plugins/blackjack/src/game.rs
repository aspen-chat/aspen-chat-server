//! The game, apart from the host: a table's seats, hands, shoe, and the chips it owes and is owed.
//!
//! The rules are a common casino's: six decks, the dealer stands on soft 17 and checks for
//! blackjack when showing an ace or a ten, blackjack pays 3 to 2, any first two cards may be
//! doubled, a pair of equal value may be split once (doubling after it allowed), and split aces
//! take one card each. There is no insurance or surrender.
//!
//! Chips are kept apart from the table, in each player's wallet, and the two cannot be written
//! together. So the table records every movement of chips it causes as a [`Transfer`], first
//! pending, and what the transfer pays for (a seat, a double, a split) waits for it: the wallet
//! applies the transfer under its id at most once, and [`Table::resolve`] then completes or
//! abandons what waited on it. A call cut off between the two leaves the transfer pending, and
//! the next call to settle the table's transfers finishes it with the same outcome.

use serde::{Deserialize, Serialize};

pub const DECKS: usize = 6;
pub const SEATS: usize = 5;
pub const MIN_BET: i64 = 10;
pub const MAX_BET: i64 = 500;
/// Bets are a multiple of this, which keeps a blackjack's 3 to 2 a whole number of chips.
pub const BET_STEP: i64 = 10;
/// A new shoe is shuffled before a round that would start with fewer cards than this left.
pub const RESHUFFLE_BELOW: usize = DECKS * 52 / 4;
/// How long after the first bet the cards are dealt, unless everyone seated says to deal sooner.
pub const BETTING_MS: i64 = 15_000;
/// How long a player has for each decision before their hand stands.
pub const TURN_MS: i64 = 30_000;
/// The most pending transfers a table keeps before it refuses to start another round.
pub const MAX_PENDING: usize = 100;

/// A source of randomness for shuffling.
pub trait Random {
    /// A number in `0..n`, every one equally likely.
    fn below(&mut self, n: usize) -> usize;
}

/// A card, numbered `suit * 13 + rank`: rank 0 is an ace and 12 a king, and the suits are, in
/// order, spades, hearts, diamonds, and clubs.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Card(pub u8);

impl Card {
    /// Its rank, 1 for an ace to 13 for a king.
    pub fn rank(self) -> u8 {
        self.0 % 13 + 1
    }

    /// What it counts for, an ace as 1.
    pub fn value(self) -> u8 {
        self.rank().min(10)
    }
}

/// A shuffled shoe of `DECKS` decks.
fn shuffled(random: &mut impl Random) -> Vec<Card> {
    let mut shoe: Vec<Card> = (0..DECKS).flat_map(|_| (0..52).map(Card)).collect();
    for i in (1..shoe.len()).rev() {
        shoe.swap(i, random.below(i + 1));
    }
    shoe
}

/// What cards count for together, and whether an ace in them counts as 11.
pub fn total(cards: &[Card]) -> (u8, bool) {
    let hard: u8 = cards.iter().map(|c| c.value()).sum();
    if hard <= 11 && cards.iter().any(|c| c.rank() == 1) {
        (hard + 10, true)
    } else {
        (hard, false)
    }
}

fn is_blackjack(cards: &[Card]) -> bool {
    cards.len() == 2 && total(cards).0 == 21
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    Blackjack,
    Win,
    Push,
    Lose,
    Bust,
}

/// A decision that waits on its chips.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Waiting {
    Double,
    Split,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hand {
    pub cards: Vec<Card>,
    pub stake: i64,
    #[serde(default)]
    pub doubled: bool,
    /// One of the two hands a split made.
    #[serde(default)]
    pub split: bool,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub outcome: Option<Outcome>,
    #[serde(default)]
    pub waiting: Option<Waiting>,
}

impl Hand {
    fn new(stake: i64) -> Self {
        Hand {
            cards: Vec::new(),
            stake,
            doubled: false,
            split: false,
            done: false,
            outcome: None,
            waiting: None,
        }
    }

    fn natural(&self) -> bool {
        !self.split && is_blackjack(&self.cards)
    }

    fn bust(&self) -> bool {
        total(&self.cards).0 > 21
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Seat {
    pub user: String,
    pub bet: i64,
    /// Whether the bet's chips have been taken from the player's wallet.
    pub confirmed: bool,
    /// Whether the player has said to deal without waiting out the betting.
    pub ready: bool,
    pub hands: Vec<Hand>,
}

/// What a transfer pays for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "for", rename_all = "camelCase")]
pub enum Purpose {
    Bet,
    Double { hand: usize },
    Split,
    Refund,
    Payout,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferState {
    Pending,
    Applied,
    Refused,
}

/// Chips moving between the table and a player's wallet: taken when `amount` is negative, paid
/// when positive. Its id is unique at its table; the wallet knows it by the table's channel too.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    pub id: String,
    pub user: String,
    pub amount: i64,
    pub purpose: Purpose,
    pub state: TransferState,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "name", rename_all = "camelCase")]
pub enum Phase {
    /// Taking bets, until `deadline` (from the first bet) or everyone seated is ready.
    Betting { deadline: Option<i64> },
    /// Waiting on one hand's decision until `deadline`.
    Playing {
        seat: usize,
        hand: usize,
        deadline: i64,
    },
    /// Every hand has been played; the dealer plays next.
    Dealer,
    /// The round is over and its hands are shown until someone bets on the next.
    Settled,
}

/// Why something was not done. Each is a key of the plugin's `messages`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    NotNow,
    Seated,
    NotSeated,
    TableFull,
    BetRange,
    NotYourTurn,
    CannotDouble,
    CannotSplit,
    Stale,
    Busy,
}

impl Refusal {
    pub fn key(self) -> &'static str {
        match self {
            Refusal::NotNow => "notNow",
            Refusal::Seated => "seated",
            Refusal::NotSeated => "notSeated",
            Refusal::TableFull => "tableFull",
            Refusal::BetRange => "betRange",
            Refusal::NotYourTurn => "notYourTurn",
            Refusal::CannotDouble => "cannotDouble",
            Refusal::CannotSplit => "cannotSplit",
            Refusal::Stale => "stale",
            Refusal::Busy => "busy",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    Hit,
    Stand,
    Double,
    Split,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Table {
    /// Counts every change, so a player's decision names the table it was made at.
    pub version: u64,
    pub round: u64,
    pub phase: Phase,
    pub seats: Vec<Seat>,
    pub dealer: Vec<Card>,
    pub shoe: Vec<Card>,
    pub ledger: Vec<Transfer>,
}

impl Default for Table {
    fn default() -> Self {
        Table {
            version: 0,
            round: 1,
            phase: Phase::Betting { deadline: None },
            seats: Vec::new(),
            dealer: Vec::new(),
            shoe: Vec::new(),
            ledger: Vec::new(),
        }
    }
}

impl Table {
    fn seat_of(&self, user: &str) -> Option<usize> {
        self.seats.iter().position(|s| s.user == user)
    }

    fn transfer(&mut self, user: &str, what: &str, amount: i64, purpose: Purpose) -> String {
        let id = format!("{}:{}:{user}:{what}", self.version, self.round);
        self.ledger.push(Transfer {
            id: id.clone(),
            user: user.to_string(),
            amount,
            purpose,
            state: TransferState::Pending,
        });
        id
    }

    /// The transfers still to be applied.
    pub fn pending(&self) -> impl Iterator<Item = &Transfer> {
        self.ledger
            .iter()
            .filter(|t| t.state == TransferState::Pending)
    }

    fn draw(&mut self, random: &mut impl Random) -> Card {
        if self.shoe.is_empty() {
            self.shoe = shuffled(random);
        }
        self.shoe.pop().unwrap_or(Card(0))
    }

    /// Starts the next round once the last is settled: the seats empty and only the transfers
    /// still pending kept.
    fn next_round(&mut self) {
        if self.phase == Phase::Settled {
            self.round += 1;
            self.seats.clear();
            self.dealer.clear();
            self.ledger.retain(|t| t.state == TransferState::Pending);
            self.phase = Phase::Betting { deadline: None };
        }
    }

    /// Seats `user` with a bet of `amount`, answering the transfer that takes it.
    pub fn bet(&mut self, user: &str, amount: i64, now: i64) -> Result<String, Refusal> {
        self.next_round();
        let Phase::Betting { deadline } = self.phase else {
            return Err(Refusal::NotNow);
        };
        if self.pending().count() >= MAX_PENDING {
            return Err(Refusal::Busy);
        }
        if self.seat_of(user).is_some() {
            return Err(Refusal::Seated);
        }
        if self.seats.len() >= SEATS {
            return Err(Refusal::TableFull);
        }
        if !(MIN_BET..=MAX_BET).contains(&amount) || amount % BET_STEP != 0 {
            return Err(Refusal::BetRange);
        }
        self.seats.push(Seat {
            user: user.to_string(),
            bet: amount,
            confirmed: false,
            ready: false,
            hands: Vec::new(),
        });
        self.phase = Phase::Betting {
            deadline: Some(deadline.unwrap_or(now + BETTING_MS)),
        };
        let id = self.transfer(user, "bet", -amount, Purpose::Bet);
        self.version += 1;
        Ok(id)
    }

    /// Takes `user`'s bet back before the deal, answering the transfer that returns it.
    pub fn leave(&mut self, user: &str) -> Result<String, Refusal> {
        if !matches!(self.phase, Phase::Betting { .. }) {
            return Err(Refusal::NotNow);
        }
        let seat = self.seat_of(user).ok_or(Refusal::NotSeated)?;
        if !self.seats[seat].confirmed {
            // Its chips are still being taken; until they are, there is nothing to give back.
            return Err(Refusal::NotNow);
        }
        let bet = self.seats.remove(seat).bet;
        if self.seats.is_empty() {
            self.phase = Phase::Betting { deadline: None };
        }
        let id = self.transfer(user, "refund", bet, Purpose::Refund);
        self.version += 1;
        Ok(id)
    }

    /// Marks `user` ready for the deal.
    pub fn ready(&mut self, user: &str) -> Result<(), Refusal> {
        if !matches!(self.phase, Phase::Betting { .. }) {
            return Err(Refusal::NotNow);
        }
        let seat = self.seat_of(user).ok_or(Refusal::NotSeated)?;
        if !self.seats[seat].ready {
            self.seats[seat].ready = true;
            self.version += 1;
        }
        Ok(())
    }

    fn all_ready(&self) -> bool {
        !self.seats.is_empty() && self.seats.iter().all(|s| s.ready && s.confirmed)
    }

    /// Records whether the wallet applied the transfer `id`, and completes or abandons what
    /// waited on it. Answers whether it changed anything: a transfer already resolved is left.
    pub fn resolve(&mut self, id: &str, applied: bool, now: i64, random: &mut impl Random) -> bool {
        let Some(transfer) = self
            .ledger
            .iter_mut()
            .find(|t| t.id == id && t.state == TransferState::Pending)
        else {
            return false;
        };
        transfer.state = if applied {
            TransferState::Applied
        } else {
            TransferState::Refused
        };
        let (user, purpose, amount) = (transfer.user.clone(), transfer.purpose, -transfer.amount);
        self.version += 1;
        let Some(seat) = self.seat_of(&user) else {
            return true;
        };
        match purpose {
            Purpose::Bet => {
                if applied {
                    self.seats[seat].confirmed = true;
                } else if matches!(self.phase, Phase::Betting { .. }) {
                    self.seats.remove(seat);
                    if self.seats.is_empty() {
                        self.phase = Phase::Betting { deadline: None };
                    }
                }
            }
            Purpose::Double { hand } => {
                let Some(h) = self.seats[seat].hands.get_mut(hand) else {
                    return true;
                };
                if h.waiting != Some(Waiting::Double) {
                    return true;
                }
                h.waiting = None;
                if applied {
                    h.stake += amount;
                    h.doubled = true;
                    let card = self.draw(random);
                    let h = &mut self.seats[seat].hands[hand];
                    h.cards.push(card);
                    h.done = true;
                }
                self.advance(now);
            }
            Purpose::Split => {
                let hands = &mut self.seats[seat].hands;
                if hands.len() != 1 || hands[0].waiting != Some(Waiting::Split) {
                    return true;
                }
                hands[0].waiting = None;
                if applied {
                    let mut second = Hand::new(amount);
                    second.split = true;
                    second.cards.push(hands[0].cards.pop().unwrap_or(Card(0)));
                    hands[0].split = true;
                    hands.push(second);
                    let aces = hands[0].cards[0].rank() == 1;
                    for index in 0..2 {
                        let card = self.draw(random);
                        let h = &mut self.seats[seat].hands[index];
                        h.cards.push(card);
                        h.done = aces || total(&h.cards).0 == 21;
                    }
                }
                self.advance(now);
            }
            Purpose::Refund | Purpose::Payout => {}
        }
        true
    }

    /// A decision on the hand whose turn it is, made at `version`. Doubling and splitting
    /// answer the transfer they wait on.
    pub fn act(
        &mut self,
        user: &str,
        version: u64,
        action: Action,
        now: i64,
        random: &mut impl Random,
    ) -> Result<Option<String>, Refusal> {
        let Phase::Playing { seat, hand, .. } = self.phase else {
            return Err(Refusal::NotNow);
        };
        if version != self.version {
            return Err(Refusal::Stale);
        }
        if self.seats[seat].user != user {
            return Err(Refusal::NotYourTurn);
        }
        if self.seats[seat].hands[hand].waiting.is_some() {
            return Err(Refusal::NotNow);
        }
        let id = match action {
            Action::Hit => {
                let card = self.draw(random);
                let h = &mut self.seats[seat].hands[hand];
                h.cards.push(card);
                h.done = total(&h.cards).0 >= 21;
                self.advance(now);
                None
            }
            Action::Stand => {
                self.seats[seat].hands[hand].done = true;
                self.advance(now);
                None
            }
            Action::Double => {
                let h = &mut self.seats[seat].hands[hand];
                if h.cards.len() != 2 || h.doubled {
                    return Err(Refusal::CannotDouble);
                }
                h.waiting = Some(Waiting::Double);
                let stake = h.stake;
                Some(self.transfer(
                    user,
                    &format!("double{hand}"),
                    -stake,
                    Purpose::Double { hand },
                ))
            }
            Action::Split => {
                let hands = &mut self.seats[seat].hands;
                if hands.len() != 1
                    || hands[0].cards.len() != 2
                    || hands[0].cards[0].value() != hands[0].cards[1].value()
                {
                    return Err(Refusal::CannotSplit);
                }
                hands[0].waiting = Some(Waiting::Split);
                let stake = hands[0].stake;
                Some(self.transfer(user, "split", -stake, Purpose::Split))
            }
        };
        self.version += 1;
        Ok(id)
    }

    /// Moves the turn to the first hand still to be played, with a fresh deadline when it
    /// moves, or to the dealer when none is.
    fn advance(&mut self, now: i64) {
        let next = self
            .seats
            .iter()
            .enumerate()
            .find_map(|(s, seat)| seat.hands.iter().position(|h| !h.done).map(|h| (s, h)));
        self.phase = match (next, self.phase) {
            (
                Some((seat, hand)),
                Phase::Playing {
                    seat: s,
                    hand: h,
                    deadline,
                },
            ) if (seat, hand) == (s, h) && self.seats[seat].hands[hand].waiting.is_some() => {
                Phase::Playing {
                    seat,
                    hand,
                    deadline,
                }
            }
            (Some((seat, hand)), _) => Phase::Playing {
                seat,
                hand,
                deadline: now + TURN_MS,
            },
            (None, _) => Phase::Dealer,
        };
    }

    /// Deals the round to everyone whose bet was taken.
    fn deal(&mut self, now: i64, random: &mut impl Random) {
        self.seats.retain(|s| s.confirmed);
        if self.seats.is_empty() {
            self.phase = Phase::Betting { deadline: None };
            return;
        }
        if self.shoe.len() < RESHUFFLE_BELOW {
            self.shoe = shuffled(random);
        }
        for seat in &mut self.seats {
            seat.hands = vec![Hand::new(seat.bet)];
        }
        self.dealer.clear();
        for _ in 0..2 {
            for seat in 0..self.seats.len() {
                let card = self.draw(random);
                self.seats[seat].hands[0].cards.push(card);
            }
            let card = self.draw(random);
            self.dealer.push(card);
        }
        let peeks = matches!(self.dealer[0].value(), 1 | 10);
        let dealer_blackjack = peeks && is_blackjack(&self.dealer);
        for seat in &mut self.seats {
            let hand = &mut seat.hands[0];
            hand.done = dealer_blackjack || hand.natural();
        }
        self.advance(now);
    }

    /// Plays the dealer's hand and settles every hand, answering nothing: the payouts are among
    /// the pending transfers.
    fn settle(&mut self, random: &mut impl Random) {
        let dealer_blackjack = is_blackjack(&self.dealer);
        let anyone_standing = self
            .seats
            .iter()
            .flat_map(|s| &s.hands)
            .any(|h| !h.bust() && !h.natural());
        if !dealer_blackjack && anyone_standing {
            while total(&self.dealer).0 < 17 {
                let card = self.draw(random);
                self.dealer.push(card);
            }
        }
        let dealer = total(&self.dealer).0;
        let mut payouts = Vec::new();
        for seat in &mut self.seats {
            for (index, hand) in seat.hands.iter_mut().enumerate() {
                let player = total(&hand.cards).0;
                let outcome = if hand.bust() {
                    Outcome::Bust
                } else if hand.natural() && !dealer_blackjack {
                    Outcome::Blackjack
                } else if dealer_blackjack {
                    if hand.natural() {
                        Outcome::Push
                    } else {
                        Outcome::Lose
                    }
                } else if dealer > 21 || player > dealer {
                    Outcome::Win
                } else if player == dealer {
                    Outcome::Push
                } else {
                    Outcome::Lose
                };
                hand.outcome = Some(outcome);
                let paid = match outcome {
                    Outcome::Blackjack => hand.stake + hand.stake * 3 / 2,
                    Outcome::Win => hand.stake * 2,
                    Outcome::Push => hand.stake,
                    Outcome::Lose | Outcome::Bust => 0,
                };
                if paid > 0 {
                    payouts.push((seat.user.clone(), index, paid));
                }
            }
        }
        for (user, index, paid) in payouts {
            self.transfer(&user, &format!("pay{index}"), paid, Purpose::Payout);
        }
        self.phase = Phase::Settled;
    }

    /// When the table next needs the host's attention: now, while transfers are pending (some
    /// can be applied only outside a player's request) or the dealer is to play, at the end of
    /// the betting or of a turn, or never.
    pub fn next_due(&self, now: i64) -> Option<i64> {
        if self.pending().next().is_some() {
            return Some(now);
        }
        match self.phase {
            Phase::Betting {
                deadline: Some(deadline),
            } => Some(if self.all_ready() { now } else { deadline }),
            Phase::Betting { deadline: None } | Phase::Settled => None,
            Phase::Playing { deadline, .. } => Some(deadline),
            Phase::Dealer => Some(now),
        }
    }

    /// Does what has fallen due: deals once the betting is over, stands a hand whose time ran
    /// out, or plays the dealer. Answers whether it did anything.
    pub fn step(&mut self, now: i64, random: &mut impl Random) -> bool {
        match self.phase {
            Phase::Betting {
                deadline: Some(deadline),
            } if (now >= deadline || self.all_ready())
                && self.seats.iter().all(|s| s.confirmed) =>
            {
                self.deal(now, random);
            }
            Phase::Playing {
                seat,
                hand,
                deadline,
            } if now >= deadline && self.seats[seat].hands[hand].waiting.is_none() => {
                self.seats[seat].hands[hand].done = true;
                self.advance(now);
            }
            Phase::Dealer => self.settle(random),
            _ => return false,
        }
        self.version += 1;
        true
    }

    /// What everyone at the table may see: everything but the shoe, the dealer's second card
    /// until the dealer plays, and the transfers.
    pub fn public(&self) -> PublicTable {
        let revealed = self.phase == Phase::Settled;
        let mut dealer: Vec<Option<Card>> = self.dealer.iter().copied().map(Some).collect();
        if !revealed && dealer.len() > 1 {
            dealer[1] = None;
        }
        PublicTable {
            version: self.version,
            round: self.round,
            phase: self.phase,
            seats: self.seats.clone(),
            dealer_total: revealed.then(|| total(&self.dealer).0),
            dealer,
            shoe_left: self.shoe.len(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicTable {
    pub version: u64,
    pub round: u64,
    pub phase: Phase,
    pub seats: Vec<Seat>,
    /// The dealer's cards, a card not yet shown as `null`.
    pub dealer: Vec<Option<Card>>,
    pub dealer_total: Option<u8>,
    pub shoe_left: usize,
}

/// The chips every player starts with, and is topped up to each day they have fewer.
pub const START_CHIPS: i64 = 1000;
/// How many of the transfers it applied a wallet remembers.
const REMEMBERED: usize = 64;

/// A player's chips in one community.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Wallet {
    pub chips: i64,
    /// The day, counted from the epoch in UTC, it was last topped up.
    pub day: i64,
    /// The latest transfers it was asked to apply, and whether it did.
    #[serde(default)]
    pub recent: Vec<(String, bool)>,
}

impl Wallet {
    pub fn new(day: i64) -> Self {
        Wallet {
            chips: START_CHIPS,
            day,
            recent: Vec::new(),
        }
    }

    /// Gives a player who has fallen below `START_CHIPS` that many again, once a day.
    pub fn top_up(&mut self, day: i64) {
        if day > self.day {
            self.chips = self.chips.max(START_CHIPS);
            self.day = day;
        }
    }

    /// Applies a transfer unless it would leave fewer than no chips, answering whether it did.
    /// The same transfer asked again is answered as it was the first time.
    pub fn apply(&mut self, id: &str, amount: i64) -> bool {
        if let Some((_, applied)) = self.recent.iter().find(|(seen, _)| seen == id) {
            return *applied;
        }
        let applied = self.chips + amount >= 0;
        if applied {
            self.chips += amount;
        }
        self.recent.push((id.to_string(), applied));
        if self.recent.len() > REMEMBERED {
            self.recent.remove(0);
        }
        applied
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deals the shoe's cards in the order given, then whatever an unshuffled shoe holds.
    struct Stacked;

    impl Random for Stacked {
        fn below(&mut self, n: usize) -> usize {
            n - 1
        }
    }

    fn card(rank: u8) -> Card {
        Card(rank - 1)
    }

    /// A table whose shoe deals `cards` first.
    fn stacked(cards: &[u8]) -> Table {
        let mut table = Table::default();
        table.shoe = shuffled(&mut Stacked);
        table.shoe.extend(cards.iter().rev().map(|&r| card(r)));
        table
    }

    fn seat(table: &mut Table, user: &str, bet: i64) {
        let id = table.bet(user, bet, 0).unwrap();
        assert!(table.resolve(&id, true, 0, &mut Stacked));
    }

    fn deal(table: &mut Table) {
        for seat in &mut table.seats {
            seat.ready = true;
        }
        assert_eq!(table.next_due(0), Some(0));
        assert!(table.step(0, &mut Stacked));
    }

    fn payouts(table: &Table) -> Vec<(String, i64)> {
        table
            .pending()
            .filter(|t| t.purpose == Purpose::Payout)
            .map(|t| (t.user.clone(), t.amount))
            .collect()
    }

    #[test]
    fn totals_count_an_ace_as_eleven_when_it_fits() {
        assert_eq!(total(&[card(1), card(6)]), (17, true));
        assert_eq!(total(&[card(1), card(6), card(10)]), (17, false));
        assert_eq!(total(&[card(1), card(1), card(9)]), (21, true));
        assert_eq!(total(&[card(13), card(12), card(2)]), (22, false));
    }

    #[test]
    fn a_shuffled_shoe_holds_six_of_each_card() {
        let shoe = shuffled(&mut Stacked);
        assert_eq!(shoe.len(), DECKS * 52);
        for c in 0..52 {
            assert_eq!(shoe.iter().filter(|x| x.0 == c).count(), DECKS);
        }
    }

    #[test]
    fn a_win_pays_double_and_the_dealer_stands_on_soft_seventeen() {
        // Player 10, 9; dealer 6, ace (soft 17).
        let mut table = stacked(&[10, 6, 9, 1]);
        seat(&mut table, "a", 100);
        deal(&mut table);
        let version = table.version;
        table
            .act("a", version, Action::Stand, 0, &mut Stacked)
            .unwrap();
        assert_eq!(table.phase, Phase::Dealer);
        assert!(table.step(0, &mut Stacked));
        assert_eq!(table.dealer.len(), 2);
        assert_eq!(table.seats[0].hands[0].outcome, Some(Outcome::Win));
        assert_eq!(payouts(&table), vec![("a".to_string(), 200)]);
    }

    #[test]
    fn a_blackjack_pays_three_to_two_without_waiting_for_a_turn() {
        // Player ace, king; dealer 9, 7.
        let mut table = stacked(&[1, 9, 13, 7]);
        seat(&mut table, "a", 100);
        deal(&mut table);
        assert_eq!(table.phase, Phase::Dealer);
        table.step(0, &mut Stacked);
        assert_eq!(table.seats[0].hands[0].outcome, Some(Outcome::Blackjack));
        // Nobody needs the dealer to draw.
        assert_eq!(table.dealer.len(), 2);
        assert_eq!(payouts(&table), vec![("a".to_string(), 250)]);
    }

    #[test]
    fn the_dealer_checks_for_blackjack_and_a_player_blackjack_pushes() {
        // Players a (10, 9) and b (ace, queen); dealer ace, king.
        let mut table = stacked(&[10, 1, 1, 9, 12, 13]);
        seat(&mut table, "a", 100);
        seat(&mut table, "b", 50);
        deal(&mut table);
        assert_eq!(table.phase, Phase::Dealer);
        table.step(0, &mut Stacked);
        assert_eq!(table.seats[0].hands[0].outcome, Some(Outcome::Lose));
        assert_eq!(table.seats[1].hands[0].outcome, Some(Outcome::Push));
        assert_eq!(payouts(&table), vec![("b".to_string(), 50)]);
    }

    #[test]
    fn a_double_waits_on_its_chips_then_takes_one_card() {
        // Player 5, 6, then 10; dealer 10, 7.
        let mut table = stacked(&[5, 10, 6, 7, 10]);
        seat(&mut table, "a", 100);
        deal(&mut table);
        let id = table
            .act("a", table.version, Action::Double, 0, &mut Stacked)
            .unwrap()
            .unwrap();
        assert!(matches!(table.phase, Phase::Playing { .. }));
        // The turn's deadline passing does not stand a hand waiting on its chips.
        assert!(!table.step(TURN_MS, &mut Stacked));
        assert!(table.resolve(&id, true, 0, &mut Stacked));
        assert!(!table.resolve(&id, true, 0, &mut Stacked));
        let hand = &table.seats[0].hands[0];
        assert_eq!((hand.cards.len(), hand.stake, hand.done), (3, 200, true));
        table.step(0, &mut Stacked);
        assert_eq!(payouts(&table), vec![("a".to_string(), 400)]);
    }

    #[test]
    fn a_double_refused_its_chips_leaves_the_hand_to_play() {
        let mut table = stacked(&[5, 10, 6, 7]);
        seat(&mut table, "a", 100);
        deal(&mut table);
        let id = table
            .act("a", table.version, Action::Double, 0, &mut Stacked)
            .unwrap()
            .unwrap();
        table.resolve(&id, false, 0, &mut Stacked);
        let hand = &table.seats[0].hands[0];
        assert_eq!((hand.cards.len(), hand.stake, hand.done), (2, 100, false));
        assert!(matches!(table.phase, Phase::Playing { .. }));
    }

    #[test]
    fn split_aces_take_one_card_each() {
        // Player ace, ace, then 10 and 9; dealer 9, 8.
        let mut table = stacked(&[1, 9, 1, 8, 10, 9]);
        seat(&mut table, "a", 100);
        deal(&mut table);
        let id = table
            .act("a", table.version, Action::Split, 0, &mut Stacked)
            .unwrap()
            .unwrap();
        table.resolve(&id, true, 0, &mut Stacked);
        let hands = &table.seats[0].hands;
        assert_eq!(hands.len(), 2);
        assert!(hands.iter().all(|h| h.done && h.cards.len() == 2));
        assert_eq!(table.phase, Phase::Dealer);
        table.step(0, &mut Stacked);
        // 21 after a split is not a blackjack: it pays even money.
        assert_eq!(table.seats[0].hands[0].outcome, Some(Outcome::Win));
        assert_eq!(
            payouts(&table),
            vec![("a".to_string(), 200), ("a".to_string(), 200)]
        );
    }

    #[test]
    fn a_decision_names_the_table_it_was_made_at() {
        let mut table = stacked(&[10, 10, 2, 7, 3]);
        seat(&mut table, "a", 100);
        deal(&mut table);
        let seen = table.version;
        table.act("a", seen, Action::Hit, 0, &mut Stacked).unwrap();
        // The same tap arriving twice hits once.
        assert_eq!(
            table.act("a", seen, Action::Hit, 0, &mut Stacked),
            Err(Refusal::Stale)
        );
        assert_eq!(table.seats[0].hands[0].cards.len(), 3);
    }

    #[test]
    fn only_the_player_whose_turn_it_is_decides() {
        let mut table = stacked(&[10, 10, 10, 7, 8, 9]);
        seat(&mut table, "a", 100);
        seat(&mut table, "b", 100);
        deal(&mut table);
        assert_eq!(
            table.act("b", table.version, Action::Stand, 0, &mut Stacked),
            Err(Refusal::NotYourTurn)
        );
    }

    #[test]
    fn a_turn_that_runs_out_stands() {
        let mut table = stacked(&[10, 10, 8, 7]);
        seat(&mut table, "a", 100);
        deal(&mut table);
        assert!(!table.step(TURN_MS - 1, &mut Stacked));
        assert!(table.step(TURN_MS, &mut Stacked));
        assert_eq!(table.phase, Phase::Dealer);
    }

    #[test]
    fn a_refused_bet_gives_up_its_seat() {
        let mut table = Table::default();
        let id = table.bet("a", 100, 0).unwrap();
        assert_eq!(table.bet("a", 100, 0), Err(Refusal::Seated));
        table.resolve(&id, false, 0, &mut Stacked);
        assert!(table.seats.is_empty());
        assert_eq!(table.phase, Phase::Betting { deadline: None });
        assert_eq!(table.bet("b", 15, 0), Err(Refusal::BetRange));
    }

    #[test]
    fn the_deal_waits_for_bets_still_being_taken() {
        let mut table = Table::default();
        table.bet("a", 100, 0).unwrap();
        table.seats[0].ready = true;
        assert!(!table.step(BETTING_MS, &mut Stacked));
    }

    #[test]
    fn leaving_before_the_deal_refunds_the_bet() {
        let mut table = Table::default();
        seat(&mut table, "a", 100);
        let id = table.leave("a").unwrap();
        let refund = table.ledger.iter().find(|t| t.id == id).unwrap();
        assert_eq!(refund.amount, 100);
        assert!(table.seats.is_empty());
    }

    #[test]
    fn the_next_bet_after_a_round_starts_another() {
        let mut table = stacked(&[10, 6, 9, 1]);
        seat(&mut table, "a", 100);
        deal(&mut table);
        table
            .act("a", table.version, Action::Stand, 0, &mut Stacked)
            .unwrap();
        table.step(0, &mut Stacked);
        let paid = payouts(&table)[0].clone();
        table.bet("b", 50, 0).unwrap();
        assert_eq!(table.round, 2);
        assert_eq!(table.seats.len(), 1);
        // The last round's payout is still owed.
        assert!(
            table
                .pending()
                .any(|t| t.user == paid.0 && t.amount == paid.1)
        );
    }

    #[test]
    fn the_hole_card_stays_hidden_until_the_dealer_plays() {
        let mut table = stacked(&[10, 6, 9, 1]);
        seat(&mut table, "a", 100);
        deal(&mut table);
        let shown = table.public();
        assert_eq!(shown.dealer, vec![Some(card(6)), None]);
        assert_eq!(shown.dealer_total, None);
        let json = serde_json::to_value(&shown).unwrap();
        assert!(json.get("shoe").is_none() && json.get("ledger").is_none());
    }

    #[test]
    fn transfer_ids_are_never_used_twice_at_a_table() {
        let mut table = Table::default();
        let first = table.bet("a", 100, 0).unwrap();
        table.resolve(&first, true, 0, &mut Stacked);
        let refund = table.leave("a").unwrap();
        let second = table.bet("a", 100, 0).unwrap();
        assert_ne!(first, second);
        assert_ne!(refund, second);
    }

    #[test]
    fn a_wallet_applies_a_transfer_once() {
        let mut wallet = Wallet::new(0);
        assert!(wallet.apply("t1", -400));
        assert!(wallet.apply("t1", -400));
        assert_eq!(wallet.chips, 600);
        assert!(!wallet.apply("t2", -700));
        assert_eq!(wallet.chips, 600);
        wallet.apply("t3", 500);
        // Asked again, a refusal stays a refusal though the chips are there now.
        assert!(!wallet.apply("t2", -700));
        assert_eq!(wallet.chips, 1100);
    }

    #[test]
    fn a_wallet_is_topped_up_once_a_day() {
        let mut wallet = Wallet::new(0);
        wallet.apply("t", -900);
        wallet.top_up(0);
        assert_eq!(wallet.chips, 100);
        wallet.top_up(1);
        assert_eq!(wallet.chips, START_CHIPS);
        wallet.apply("u", 500);
        wallet.top_up(2);
        assert_eq!(wallet.chips, 1500);
    }
}
