use std::cell::{RefCell};
use std::future::Future;
use std::sync::{Weak};
use threadpool::THREAD_POOL;

#[cfg(test)]
mod tests;
mod threadpool;
mod task;
mod atomics;
mod raw;

use task::{Task, TaskQueue};

pub const WAITING: usize = 0; // --> POLLING
pub const POLLING: usize = 1; // --> WAITING, REPOLL, or COMPLETE
pub const REPOLL: usize = 2; // --> POLLING
pub const COMPLETE: usize = 3; // No transitions out

thread_local! {
    static QUEUE: RefCell<Weak<TaskQueue>> = RefCell::new(Weak::new());
}

#[inline]
pub fn spawn<F>(future: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    QUEUE.with(|q| {
        if let Some(q) = q.borrow().upgrade() {
            q.tx.send(Task::new(future, q.clone())).unwrap();
        } else {
            THREAD_POOL.spawn(future);
        }
    });
}
