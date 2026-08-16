//! Bitwark — a classical chess engine speaking UCI.
//!
//! Process architecture (the async skeleton everything else hangs on):
//!
//! ```text
//!                 stdin ──▶ UciSession ──▶ mpsc channel ──▶ WriterTask ──▶ stdout
//!                            (parses & dispatches)         (single owner of stdout)
//! ```
//!
//! Why this shape?
//!
//! * UCI is *line-oriented*: the GUI sends one command per line and expects
//!   one-line replies. A dedicated writer task that owns stdout guarantees
//!   lines never interleave, no matter how many tasks/threads produce them.
//! * Search runs on its own synchronous *OS thread* (not a tokio task —
//!   search is pure CPU work and must not occupy async workers). That thread
//!   will feed `info`/`bestmove` lines into the same channel, so it can never
//!   race the session's protocol replies.
//! * The session loop stays free to answer `isready` with `readyok` even
//!   while a search is running (a hard UCI requirement).
//!
//! The runtime uses 2 worker threads: the session task awaits stdin, the
//! writer task awaits the channel — both are idle 99.99% of the time.

mod uci;

#[tokio::main(worker_threads = 2)]
async fn main() {
    // Bounded channel: if the GUI stops reading our output, we block instead
    // of buffering without limit (back-pressure beats OOM).
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(1024);

    let writer = tokio::spawn(uci::session::writer_task(rx));
    let mut session = uci::session::UciSession::new(tx);

    let result = session.run().await;

    // Dropping the session closes the sender half, which lets the writer
    // task drain its remaining lines and exit cleanly.
    drop(session);
    let _ = writer.await;

    // A stdin read error is the only failure mode here; treat it like `quit`.
    std::process::exit(if result.is_ok() { 0 } else { 1 });
}
