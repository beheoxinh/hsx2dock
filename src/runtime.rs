use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};

use tokio::runtime::Runtime;

static RT: OnceLock<Runtime> = OnceLock::new();

/// The shared Tokio runtime for all background Docker work. The GTK main loop
/// is not a Tokio executor, so background Docker work gets dispatched here and
/// results are handed back through `async_channel`.
pub fn runtime() -> &'static Runtime {
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .thread_name("docker-manager-rt")
            .build()
            .expect("failed to build shared Tokio runtime")
    })
}

/// Spawn a background future on the shared runtime.
pub fn spawn<F>(future: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    runtime().spawn(future);
}

type Job = Box<dyn FnOnce() + Send + 'static>;

/// A single OS thread that lives for the whole process, dedicated to engine
/// lifecycle ops (`start`/`stop`).
///
/// `EngineManager::start` installs `PR_SET_PDEATHSIG` in a `pre_exec` hook so
/// the daemon dies with the app. That signal is delivered when the *thread that
/// forked it* exits — so the fork must happen on a thread that never exits.
/// Tokio's blocking pool recycles idle threads (default `thread_keep_alive`
/// 10s), which killed a freshly-booted daemon ~10s after start. This thread
/// never exits, so the parent stays valid for the app's lifetime.
fn engine_thread() -> &'static Mutex<Sender<Job>> {
    static T: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    T.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        std::thread::Builder::new()
            .name("docker-engine".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    job();
                }
            })
            .expect("failed to spawn persistent engine thread");
        Mutex::new(tx)
    })
}

/// Run `f` on the persistent engine thread; await its result on the caller.
///
/// Use this for anything that forks the engine process (start/stop), never
/// `spawn_blocking` — see [`engine_thread`].
pub fn run_engine_op<F, T>(f: F) -> async_channel::Receiver<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = async_channel::bounded(1);
    let job: Job = Box::new(move || {
        let _ = tx.send_blocking(f());
    });
    engine_thread()
        .lock()
        .unwrap()
        .send(job)
        .expect("engine thread is gone");
    rx
}

/// Awaitable form of [`run_engine_op`].
pub async fn engine_op<F, T>(f: F) -> Result<T, String>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    run_engine_op(f)
        .recv()
        .await
        .map_err(|e| format!("engine thread: {e}"))
}
