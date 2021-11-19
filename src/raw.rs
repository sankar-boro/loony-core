use std::mem::{ManuallyDrop};
use std::sync::{
    atomic::{Ordering::SeqCst},
    Arc,
};
use std::task::{RawWaker, RawWakerVTable, Waker};

use crate::{WAITING, POLLING};
use crate::atomics::AtomicFuture;
use super::task::{task, clone_task};

#[inline]
pub unsafe fn waker(task: *const AtomicFuture) -> Waker {
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