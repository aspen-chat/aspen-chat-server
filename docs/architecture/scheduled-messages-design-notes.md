# Scheduled messages: design notes

Why [scheduled messages](scheduled-messages.md) are shaped as they are.

- **A scheduled message is a held message, not a table of its own.** A held message already is a
  message checked now and posted later by a job: posted with the permissions and plugins of that
  moment, in the language it was written in, exactly once however many servers try, dropped when its
  author leaves, its files kept from the unsent sweep, its author's other devices told. A second
  table would have needed each of those again, and each is a place for the two to drift apart.
- **`send_at` marks a scheduled row, not `waiting`.** `waiting` says what the row waits for now, and
  a scheduled message waits for mana like anything else once its time has come. What must outlast
  that move (never dropped for waiting, failed rather than dropped, shown in its author's list) goes
  by `send_at`.
- **A failed one is kept; anything else held is dropped.** Someone whose message is held sent it
  moments ago and is looking at it: told it was dropped, they send it again. Someone whose scheduled
  message fails is likely away, and a notice that passes while they are gone would lose what they
  wrote. So it stays in their list with the reason until they act.
- **Unless they can no longer see the channel.** The list is reached from the channel's message box.
  A failed message in a channel its author cannot open could never be found or deleted, and would
  count toward their hundred for good, so there it is dropped as a held message is.
- **Failed ones are swept after thirty days.** The ways of losing sight of a channel after a message
  failed there are many (an override, a role, a move, a deletion), and none of them looks for failed
  messages. A sweep by age bounds what any of them can strand, and lets the files go.
- **It never expires from the mana queue.** The two-minute limit drops what a busy channel cannot
  post soon, and its author sends it again when the channel is quieter. Nobody is there to send a
  scheduled message again, so it waits its turn however long that takes. A raid that schedules
  messages rather than sending them gains queue places that do not expire, but no more of them: an
  author's item and the queue's items are bounded as before, the queue still orders by rank and
  tenure, and room is still made for anyone above them.
- **Nor does want of room fail it.** A full item or a full queue, or being displaced from one, is a
  passing state of a busy channel, and the message is unattended: nobody would send it again, so it
  tries again every ten seconds until it gets in. Only what will never pass (a lost permission, a
  refusal) fails it.
- **Another time replaces the job rather than moving it.** A job's `not_before` is also its lease:
  the runner pushes it forward while a step runs and sets it again when the step ends, so a time
  written by a request while a step ran would be overwritten. Deleting the job and saving a new one
  in the transaction that changes `send_at` leaves no moment when a scheduled message has no job,
  and the old job's step, if one is running, finds the row changed and does nothing.
- **A time can change while it waits for mana.** The drainer reads a row and posts it a moment
  later, with nothing held between, but posting takes a row only while it is still due, so a row
  given a later time under the drainer is simply not posted, and leaves the queue. Nothing is lost
  but, at worst, the mana just paid for it.
- **Sending now goes through the job.** One path posts every scheduled message, so sending now and a
  time coming due cannot differ: the request makes it due, and what becomes of it (posted, waiting
  for mana, failed) reaches the author the way it would have at its time.
- **Editing is deleting and writing again.** The message box is where a message is written, with its
  tags, files, and reply; a second editor for scheduled messages would repeat it. Taking one back
  into the box deletes it on the server first, so what is being changed cannot be posted meanwhile,
  and its files are counted as confirmed afresh so the day an unsent upload is kept starts again.
- **One event with the whole record.** The record is small and changes rarely. Carrying it whole
  makes the event idempotent under replay, and no field can be left out of an update.
- **The limits are constants.** A hundred per person and two years ahead bound the rows, the jobs,
  and the files held, and nothing about a deployment's size or policy argues for other numbers.
- **The unsent sweep walks each old upload once.** Files in scheduled messages stay unsent, and
  older than a day, for as long as two years, so a sweep that took the first batch of old unsent
  rows again and again would reread them with every batch. It walks them in order of confirmation
  from where the last batch ended instead: each sweep reads each such file once.
- **The date field is React Aria's, not the browser's.** The browser's `datetime-local` would give
  every platform its own picker with no dependency, but it writes the date in the browser's
  language, not the one the app shows; React Aria's segments follow the app's locale like every
  other date in it, at the cost of `@internationalized/date` as a direct dependency, for the values
  it holds.
