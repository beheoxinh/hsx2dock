use std::sync::OnceLock;

use tokio::runtime::Runtime;

static RT: OnceLock<Runtime> = OnceLock::new();

/// Global multi-threaded Tokio runtime used for every bollard call.
///
/// The GTK main loop is not a Tokio executor, so background Docker work is
/// dispatched here and results are handed back through `async_channel`.
pub fn runtime() -> &'static Runtime {
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .thread_name("docker-manager-rt")
            .build()
            .expect("failed to build tokio runtime")
    })
}

/// Spawn a fire-and-forget future on the shared runtime.
pub fn spawn<F>(future: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    runtime().spawn(future);
}
