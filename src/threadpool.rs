use std::cell::{RefCell};
use std::future::Future;
use std::sync::{
    Arc, Weak,
};
use std::thread;

use crossbeam::channel;
use futures::future::BoxFuture;
use crate::{TaskQueue, Task};

thread_local! {
    static QUEUE: RefCell<Weak<TaskQueue>> = RefCell::new(Weak::new());
}

lazy_static::lazy_static! {
    pub static ref THREAD_POOL: ThreadPool = ThreadPool::new();
}

/// A threadpool that futures can be spawned on.
///
/// This is useful when you want to perform some setup logic around the
/// threadpool. If you don't need to setup extra logic, it's recommended to use
/// `loony_core::spawn()` directly.
pub struct ThreadPool {
    queue: Arc<TaskQueue>,
}

impl ThreadPool {
    /// Create a new threadpool instance.
    #[inline]
    pub fn new() -> Self {
        Self::with_setup()
    }

    /// Create a new instance with a method that's called for every thread
    /// that's spawned.
    #[inline]
    pub fn with_setup() -> Self
    // where
        // F: Fn() + Send + Sync + 'static,
    {
        // let f = Arc::new(f);
        let (tx, rx) = channel::unbounded();
        let queue = Arc::new(TaskQueue { tx, rx });
        let max_cpus = num_cpus::get() * 2;
        for _ in 0..max_cpus {
            // let f = f.clone();
            let rx = queue.rx.clone();
            let queue = Arc::downgrade(&queue);
            thread::spawn(move || {
                QUEUE.with(|q| *q.borrow_mut() = queue.clone());
                // f();
                for task in rx {
                    unsafe { task.poll() }
                }
            });
        }
        ThreadPool { queue }
    }

    /// Spawn a new future on the threadpool.
    #[inline]
    pub fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.queue
            .tx
            .send(Task::new(future, self.queue.clone()))
            .unwrap();
    }

    /// Spawn a boxed future on the threadpool.
    #[inline]
    pub fn spawn_boxed(&self, future: BoxFuture<'static, ()>) {
        self.queue
            .tx
            .send(Task::new_boxed(future, self.queue.clone()))
            .unwrap();
    }
}