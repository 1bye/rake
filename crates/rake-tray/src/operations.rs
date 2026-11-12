//! Local host-owned operation observation, independent of caller receipt lifetime.
use crate::{Error, Request};
use futures::{FutureExt, StreamExt, future::LocalBoxFuture, stream::FuturesUnordered};

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
};

struct Pending {
    future: LocalBoxFuture<'static, ()>,
    completed: Rc<Cell<bool>>,
}

#[derive(Default)]
pub(crate) struct Operations {
    pending: RefCell<VecDeque<Pending>>,
    in_flight: Cell<usize>,
    closed: Cell<bool>,
    changed: event_listener::Event,
}

impl Operations {
    pub(crate) fn capacity(&self) -> Result<(), Error> {
        if self.closed.get() {
            return Err(Error::Stopped);
        }

        if self.in_flight.get() >= 64 {
            return Err(Error::QueueFull);
        }

        Ok(())
    }

    pub(crate) fn submit<T: Clone + 'static>(self: &Rc<Self>, future: Request<T>) -> Request<T> {
        let completed = Rc::new(Cell::new(false));
        let done = completed.clone();
        let queue = Rc::downgrade(self);

        self.in_flight.set(self.in_flight.get() + 1);
        let future = async move {
            let result = future.await;

            done.set(true);

            if let Some(queue) = queue.upgrade() {
                queue.in_flight.set(queue.in_flight.get() - 1);
                queue.changed.notify(usize::MAX);
            }

            result
        }
        .boxed_local()
        .shared();

        let observer = future.clone();
        let mut pending = self.pending.borrow_mut();

        pending.retain(|pending| !pending.completed.get());
        pending.push_back(Pending {
            future: async move {
                let _ = observer.await;
            }
            .boxed_local(),
            completed,
        });
        drop(pending);
        self.changed.notify(usize::MAX);

        Box::pin(future)
    }

    pub(crate) fn close(&self) {
        self.closed.set(true);
        self.changed.notify(usize::MAX);
    }

    pub(crate) async fn drive(&self) {
        self.run(false).await;
    }

    pub(crate) async fn flush(&self) {
        self.run(true).await;
    }

    async fn run(&self, until_idle: bool) {
        let mut active = FuturesUnordered::new();

        loop {
            let changed = self.changed.listen();

            for pending in self.pending.borrow_mut().drain(..) {
                active.push(pending.future);
            }

            if (self.closed.get() || until_idle) && self.in_flight.get() == 0 {
                return;
            }

            if active.is_empty() {
                changed.await;
            } else {
                futures::pin_mut!(changed);
                let next = active.next();

                futures::pin_mut!(next);
                futures::future::select(changed, next).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Covers: receipt cancellation and admission bounds. Real: operation queue/futures.
    //! Doubles: caller-supplied finite native work. Requires: no desktop.
    use super::*;

    #[test]
    fn dropping_receipts_does_not_cancel_admitted_work() {
        let queue = Rc::new(Operations::default());
        let applied = Rc::new(Cell::new(false));
        let effect = applied.clone();

        drop(queue.submit(Box::pin(async move {
            effect.set(true);

            Ok(())
        })));
        queue.close();
        futures::executor::block_on(queue.drive());
        assert!(applied.get());
        assert_eq!(queue.in_flight.get(), 0);
    }

    #[test]
    fn bounded_admission_is_released_when_operations_finish() {
        let queue = Rc::new(Operations::default());

        let receipts = (0..64)
            .map(|_| queue.submit(crate::ready(Ok(()))))
            .collect::<Vec<_>>();

        assert!(matches!(queue.capacity(), Err(Error::QueueFull)));
        futures::executor::block_on(futures::future::join_all(receipts));
        queue.capacity().unwrap();
        queue.close();
        futures::executor::block_on(queue.drive());
    }
}
