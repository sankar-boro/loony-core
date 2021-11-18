//! loony_core is a concurrent executor for Rust futures. It is implemented as a
//! threadpool executor using a single, shared queue. Algorithmically, it is very
//! similar to the Threadpool executor provided by the futures crate. The main
//! difference is that loony_core uses a crossbeam channel and performs a single
//! allocation per spawned future, whereas the futures Threadpool uses std
//! concurrency primitives and multiple allocations.
//!
//! Similar to [romio][romio] - an IO reactor - loony_core currently provides no user
//! configuration. It exposes the most minimal API possible.
//!
//! [romio]: https://github.com/withoutboats/romio
//!
//! ## Example
//! ```rust,no_run
//! use std::io;
//!
//! use futures::StreamExt;
//! use futures::executor;
//! use futures::io::AsyncReadExt;
//!
//! use romio::{TcpListener, TcpStream};
//!
//! fn main() -> io::Result<()> {
//!     executor::block_on(async {
//!         let mut listener = TcpListener::bind(&"127.0.0.1:7878".parse().unwrap())?;
//!         let mut incoming = listener.incoming();
//!
//!         println!("Listening on 127.0.0.1:7878");
//!
//!         while let Some(stream) = incoming.next().await {
//!             let stream = stream?;
//!             let addr = stream.peer_addr()?;
//!
//!             loony_core::spawn(async move {
//!                 println!("Accepting stream from: {}", addr);
//!
//!                 echo_on(stream).await.unwrap();
//!
//!                 println!("Closing stream from: {}", addr);
//!             });
//!         }
//!
//!         Ok(())
//!     })
//! }
//!
//! async fn echo_on(stream: TcpStream) -> io::Result<()> {
//!     let (mut reader, mut writer) = stream.split();
//!     reader.copy_into(&mut writer).await?;
//!     Ok(())
//! }
//! ```
use std::cell::{RefCell, UnsafeCell};
use std::fmt;
use std::future::Future;
use std::mem::{forget, ManuallyDrop};
use std::sync::{
    atomic::{AtomicUsize, Ordering::SeqCst},
    Arc, Weak,
};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use crossbeam::channel;
use futures::future::BoxFuture;
use futures::prelude::*;


#[cfg(test)]
mod tests;
mod threadpool;

use threadpool::THREAD_POOL;

thread_local! {
    static QUEUE: RefCell<Weak<TaskQueue>> = RefCell::new(Weak::new());
}

/// Spawn a task on the threadpool.
///
/// ## Example
/// ```rust,ignore
/// use std::thread;
/// use futures::executor;
///
/// fn main() {
///     for _ in 0..10 {
///         loony_core::spawn(async move {
///             let id = thread::current().id();
///             println!("Running on thread {:?}", id);
///         })
///     }
/// }
/// ```
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

struct TaskQueue {
    tx: channel::Sender<Task>,
    rx: channel::Receiver<Task>,
}

impl Default for TaskQueue {
    fn default() -> TaskQueue {
        let (tx, rx) = channel::unbounded();
        TaskQueue { tx, rx }
    }
}

#[derive(Clone, Debug)]
#[repr(transparent)]
struct Task(Arc<AtomicFuture>);

struct AtomicFuture {
    queue: Arc<TaskQueue>,
    status: AtomicUsize,
    future: UnsafeCell<BoxFuture<'static, ()>>,
}

impl fmt::Debug for AtomicFuture {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        "AtomicFuture".fmt(f)
    }
}

unsafe impl Send for AtomicFuture {}
unsafe impl Sync for AtomicFuture {}

const WAITING: usize = 0; // --> POLLING
const POLLING: usize = 1; // --> WAITING, REPOLL, or COMPLETE
const REPOLL: usize = 2; // --> POLLING
const COMPLETE: usize = 3; // No transitions out

impl Task {
    #[inline]
    fn new<F: Future<Output = ()> + Send + 'static>(future: F, queue: Arc<TaskQueue>) -> Task {
        let future: Arc<AtomicFuture> = Arc::new(AtomicFuture {
            queue,
            status: AtomicUsize::new(WAITING),
            future: UnsafeCell::new(future.boxed()),
        });
        let future: *const AtomicFuture = Arc::into_raw(future) as *const AtomicFuture;
        unsafe { task(future) }
    }

    #[inline]
    fn new_boxed(future: BoxFuture<'static, ()>, queue: Arc<TaskQueue>) -> Task {
        let future: Arc<AtomicFuture> = Arc::new(AtomicFuture {
            queue,
            status: AtomicUsize::new(WAITING),
            future: UnsafeCell::new(future),
        });
        let future: *const AtomicFuture = Arc::into_raw(future) as *const AtomicFuture;
        unsafe { task(future) }
    }

    fn send(&self, data: Self) {
        self.0.queue.tx.send(data).expect("Ooops! Failed to send data");
    }

    fn polling_repoll(&self) -> Result<usize, usize> {
        self
        .0
        .status
        .compare_exchange(POLLING, REPOLL, SeqCst, SeqCst)
    }

    fn waiting_repoll(&self) -> Result<usize, usize> {
        self
        .0
        .status
        .compare_exchange(WAITING, REPOLL, SeqCst, SeqCst)
    }

    fn waiting_polling(&self) -> Result<usize, usize> {
        self
        .0
        .status
        .compare_exchange(WAITING, POLLING, SeqCst, SeqCst)
    }

    fn polling_waiting(&self) -> Result<usize, usize> {
        self
        .0
        .status
        .compare_exchange(POLLING,WAITING, SeqCst, SeqCst)
    }

    #[inline]
    unsafe fn poll(self) {
        self.0.status.store(POLLING, SeqCst);
        let waker = ManuallyDrop::new(waker(&*self.0));
        let mut cx = Context::from_waker(&waker);
        loop {
            if let Poll::Ready(_) = (&mut *self.0.future.get()).poll_unpin(&mut cx) {
                break self.0.status.store(COMPLETE, SeqCst);
            }
            match self.polling_waiting()
            {
                Ok(_) => break,
                Err(_) => self.0.status.store(POLLING, SeqCst),
            }
        }
    }
}

#[inline]
unsafe fn waker(task: *const AtomicFuture) -> Waker {
    Waker::from_raw(RawWaker::new(
        task as *const (),
        &RawWakerVTable::new(clone_raw, wake_raw, wake_ref_raw, drop_raw),
    ))
}

#[inline]
unsafe fn clone_raw(this: *const ()) -> RawWaker {
    let task = clone_task(this as *const AtomicFuture);
    RawWaker::new(
        Arc::into_raw(task.0) as *const (),
        &RawWakerVTable::new(clone_raw, wake_raw, wake_ref_raw, drop_raw),
    )
}

#[inline]
unsafe fn drop_raw(this: *const ()) {
    drop(task(this as *const AtomicFuture))
}

#[inline]
unsafe fn wake_raw(this: *const ()) {
    let task = task(this as *const AtomicFuture);
    let mut status = task.0.status.load(SeqCst);
    loop {
        match status {
            WAITING => {
                match task.waiting_repoll()
                {
                    Ok(_) => {
                        task.send(clone_task(&*task.0));
                        break;
                    }
                    Err(cur) => status = cur,
                }
            }
            POLLING => {
                match task.polling_repoll()
                {
                    Ok(_) => break,
                    Err(cur) => status = cur,
                }
            }
            _ => break,
        }
    }
}

#[inline]
unsafe fn wake_ref_raw(this: *const ()) {
    let task = ManuallyDrop::new(task(this as *const AtomicFuture));
    let mut status = task.0.status.load(SeqCst);
    loop {
        match status {
            WAITING => {
                match task.waiting_polling()
                {
                    Ok(_) => {
                        task.send(clone_task(&*task.0));
                        break;
                    }
                    Err(cur) => status = cur,
                }
            }
            POLLING => {
                match task.polling_repoll()
                {
                    Ok(_) => break,
                    Err(cur) => status = cur,
                }
            }
            _ => break,
        }
    }
}

#[inline]
unsafe fn task(future: *const AtomicFuture) -> Task {
    Task(Arc::from_raw(future))
}

#[inline]
unsafe fn clone_task(future: *const AtomicFuture) -> Task {
    let task = task(future);
    forget(task.clone());
    task
}
