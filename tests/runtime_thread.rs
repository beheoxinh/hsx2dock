//! Unit tests for the persistent engine thread: `run_engine_op` must run the
//! closure on a dedicated OS thread (NOT the Tokio blocking pool) and return
//! the result to the awaiting caller.
//!
//! Regression guard: the engine child is forked with `PR_SET_PDEATHSIG`, so the
//! forking thread must never be reclaimed. Running these ops via
//! `spawn_blocking` (whose idle threads Tokio recycles) killed the daemon ~10s
//! after boot.

#[cfg(test)]
mod tests {
    use docker_manager::runtime::{engine_op, run_engine_op};

    #[test]
    fn run_engine_op_returns_value() {
        let rx = run_engine_op(|| 41 + 1);
        assert_eq!(rx.recv_blocking().expect("value"), 42);
    }

    #[tokio::test]
    async fn engine_op_awaits_result() {
        let r = engine_op(|| "ok".to_string()).await;
        assert_eq!(r.as_deref(), Ok("ok"));
    }

    #[test]
    fn engine_thread_is_dedicated() {
        // The persistent engine thread has a stable name; a Tokio blocking-pool
        // thread would be unnamed ("<unnamed>").
        let rx = run_engine_op(|| std::thread::current().name().unwrap_or("<unnamed>").to_string());
        assert_eq!(rx.recv_blocking().expect("name"), "docker-engine");
    }

    #[test]
    fn engine_thread_is_reused() {
        // Successive ops must land on the same long-lived thread, not a fresh
        // one each time — proves the thread is not being recreated per op.
        let a = run_engine_op(|| std::thread::current().id())
            .recv_blocking()
            .expect("id");
        let b = run_engine_op(|| std::thread::current().id()).recv_blocking().expect("id");
        assert_eq!(a, b);
    }
}