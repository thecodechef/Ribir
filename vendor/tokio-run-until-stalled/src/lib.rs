/// A utility for running [LocalSet] asynchronous tasks until
/// no more progress can be made on any task in the [LocalSet].
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, RawWaker, RawWakerVTable, Waker};

use tokio::task::LocalSet;

/// An extension trait for `tokio::runtime::Runtime` that provides a method for running
/// asynchronous tasks of a `LocalSet` until no more progress can be made on any task in the `LocalSet`.
///
/// This trait is implemented for `tokio::runtime::Runtime` and `tokio::runtime::Handle`.
pub trait RuntimeExt {
    /// Run the given [LocalSet] until it stalls.
    ///
    /// This method runs the given [LocalSet] until it stalls.
    /// If the [LocalSet] stalls, this method returns `None`. Otherwise,
    /// it returns `Some(LocalSet)` containing the [`LocalSet`] need to run again.
    ///
    /// # Examples
    ///
    /// ``` rust
    /// use std::time::Duration;
    /// use tokio::{runtime::Builder, time::sleep, task::LocalSet};
    /// use tokio_run_until_stalled::*;
    ///
    /// let local_set = LocalSet::new();
    /// local_set.spawn_local(async {
    ///     println!("Task 1 completed");
    /// });
    /// local_set.spawn_local(async {
    ///    sleep(Duration::from_millis(100)).await;
    ///    println!("Task 2 completed");
    /// });
    /// let rt = Builder::new_multi_thread().enable_all().build().unwrap();
    /// let result = rt.run_until_stalled(local_set);
    /// assert!(result.is_some());
    /// rt.block_on(async { sleep(Duration::from_millis(150)).await; });
    /// let final_result = rt.run_until_stalled(result.unwrap());
    /// assert!(final_result.is_none());
    /// ```
    fn run_until_stalled(&self, local_set: LocalSet) -> Option<LocalSet>;
}

/// An extension trait for `tokio::task::LocalSet` convert it to `RunUntilStalled`.
pub trait LocalSetExt {
    fn run_until_stalled(self) -> RunUntilStalled;
}

/// [RunUntilStalled] A Future for running asynchronous tasks of a `LocalSet` until no more
/// progress can be made on any task in the `LocalSet`.
/// This future is created by the [`LocalSet::run_until_stalled`] method.
/// # Examples
///
/// ``` rust
/// use std::time::Duration;
/// use tokio::{time::{sleep}, task::LocalSet};
/// use tokio_run_until_stalled::*;
///
/// #[tokio::main]
/// async fn main() {
///    let local_set = LocalSet::new();
///    local_set.spawn_local(async {
///        println!("Task 1 completed");
///    });
///    local_set.spawn_local(async {
///        sleep(Duration::from_millis(100)).await;
///        println!("Task 2 completed");
///    });
///    let mut run_stalled = local_set.run_until_stalled();
///    while !run_stalled.is_done() {
///        run_stalled = run_stalled.await;
///        sleep(Duration::from_millis(150)).await;
///    }
///}
/// ```
pub struct RunUntilStalled {
    local_set: Option<LocalSet>,
}

impl RunUntilStalled {
    /// Convert `RunUntilStalled` to `Option<LocalSet>`
    pub fn into_local_set(self) -> Option<LocalSet> {
        self.local_set
    }

    /// Check if the `RunUntilStalled` is done
    pub fn is_done(&self) -> bool {
        self.local_set.is_none()
    }

    /// Get the `LocalSet`
    pub fn as_local_set(&self) -> &Option<LocalSet> {
        &self.local_set
    }

    /// Force get the `LocalSet`
    pub fn as_local_set_force(&self) -> &LocalSet {
        self.local_set.as_ref().unwrap()
    }
}

struct DetectWaker {
    flag: AtomicBool,
    waker: Waker,
}

impl DetectWaker {
    fn wake(&self) {
        self.flag.store(true, Ordering::Release);
        self.waker.wake_by_ref();
    }

    fn has_waked(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}

fn create_detector_waker(waker: Waker) -> (Arc<DetectWaker>, Waker) {
    let detector = Arc::new(DetectWaker {
        flag: AtomicBool::default(),
        waker,
    });
    static VTABLE: RawWakerVTable = RawWakerVTable::new(
        |data| {
            let origin = unsafe { Arc::from_raw(data as *const DetectWaker) };
            let cloned = origin.clone();
            std::mem::forget(origin);
            RawWaker::new(Arc::into_raw(cloned) as *const _ as *const (), &VTABLE)
        },
        |data| {
            let waker = unsafe { Arc::from_raw(data as *const DetectWaker) };
            waker.wake();
        },
        |data| {
            let waker = unsafe { Arc::from_raw(data as *const DetectWaker) };
            waker.wake();
            std::mem::forget(waker);
        },
        |data| {
            let _ = unsafe { Arc::from_raw(data as *const DetectWaker) };
        },
    );

    let waker = unsafe {
        Waker::from_raw(RawWaker::new(
            Arc::into_raw(detector.clone()) as *const _ as *const (),
            &VTABLE,
        ))
    };
    (detector, waker)
}

impl Future for RunUntilStalled {
    type Output = Self;
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        if self.local_set.is_none() {
            return std::task::Poll::Ready(RunUntilStalled { local_set: None });
        }

        let (detector, waker) = create_detector_waker(cx.waker().clone());
        let mut new_ctx = Context::from_waker(&waker);
        let mut local_set = self.local_set.take().unwrap();
        if std::task::Poll::Ready(()) == Pin::new(&mut local_set).poll(&mut new_ctx) {
            std::task::Poll::Ready(RunUntilStalled { local_set: None })
        } else if detector.has_waked() {
            self.local_set = Some(local_set);
            cx.waker().wake_by_ref();
            std::task::Poll::Pending
        } else {
            std::task::Poll::Ready(RunUntilStalled {
                local_set: Some(local_set),
            })
        }
    }
}

impl LocalSetExt for LocalSet {
    fn run_until_stalled(self) -> RunUntilStalled {
        RunUntilStalled {
            local_set: Some(self),
        }
    }
}

impl RuntimeExt for tokio::runtime::Runtime {
    fn run_until_stalled(&self, local_set: LocalSet) -> Option<LocalSet> {
        self.block_on(local_set.run_until_stalled())
            .into_local_set()
    }
}

impl RuntimeExt for tokio::runtime::Handle {
    fn run_until_stalled(&self, local_set: LocalSet) -> Option<LocalSet> {
        self.block_on(local_set.run_until_stalled())
            .into_local_set()
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use super::*;

    use tokio::{
        sync::mpsc::channel,
        task,
        time::{sleep, Duration},
    };

    #[tokio::test]
    async fn test_run_until_stalled() {
        let local_set = LocalSet::new();
        let logs = Rc::new(RefCell::new(vec![]));
        local_set.spawn_local({
            let logger = logs.clone();
            async move {
                logger.borrow_mut().push("task 1");
                async {
                    logger.borrow_mut().push("async log in task 1");
                }
                .await;
                task::spawn_local({
                    let logger = logger.clone();
                    async move {
                        logger.borrow_mut().push("inner spawn task");
                        sleep(Duration::from_millis(100)).await;
                        logger.borrow_mut().push("inner spawn task completed");
                    }
                });
                logger.borrow_mut().push("Task 1 completed");
            }
        });

        let mut run_stalled = local_set.run_until_stalled();

        run_stalled = run_stalled.await;
        assert_eq!(
            &*logs.borrow(),
            &[
                "task 1",
                "async log in task 1",
                "Task 1 completed",
                "inner spawn task"
            ]
        );
        sleep(Duration::from_millis(150)).await;
        run_stalled = run_stalled.await;
        assert_eq!(
            &*logs.borrow(),
            &[
                "task 1",
                "async log in task 1",
                "Task 1 completed",
                "inner spawn task",
                "inner spawn task completed"
            ]
        );
        assert!(run_stalled.is_done());
    }

    #[tokio::test]
    async fn test_depend_on_each() {
        let local_set = LocalSet::new();
        let logs = Rc::new(RefCell::new(vec![]));
        let (tx1, mut rx1) = channel::<()>(1);
        let (tx2, mut rx2) = channel::<()>(1);
        local_set.spawn_local({
            let logger = logs.clone();
            async move {
                logger.borrow_mut().push("task1");
                logger.borrow_mut().push("wait for rx2");
                if let Some(_) = rx2.recv().await {
                    logger.borrow_mut().push("rx2 recv");
                    logger.borrow_mut().push("tx1 send");
                    let _ = tx1.send(()).await;
                }
                logger.borrow_mut().push("task1 completed");
            }
        });

        local_set.spawn_local({
            let logger = logs.clone();
            async move {
                logger.borrow_mut().push("task2");
                logger.borrow_mut().push("tx2 send");
                let _ = tx2.send(()).await;
                logger.borrow_mut().push("wait for rx1");
                if let Some(_) = rx1.recv().await {
                    logger.borrow_mut().push("rx1 recv");
                }
                logger.borrow_mut().push("task2 completed");
            }
        });

        let mut run_stalled = local_set.run_until_stalled();

        run_stalled = run_stalled.await;
        assert_eq!(
            &*logs.borrow(),
            &[
                "task1",
                "wait for rx2",
                "task2",
                "tx2 send",
                "wait for rx1",
                "rx2 recv",
                "tx1 send",
                "task1 completed",
                "rx1 recv",
                "task2 completed"
            ]
        );
        assert!(run_stalled.is_done());
    }
}
