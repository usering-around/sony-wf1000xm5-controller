use std::{
    cell::{Cell, Ref, RefCell},
    rc::Rc,
};

use tokio::task::JoinHandle;

/// A resource which can be acquired asynchornously (single threaded)
pub struct AsyncResource<T> {
    res: Rc<RefCell<Option<T>>>,
    need_clear: Rc<Cell<bool>>,
    handle: Rc<RefCell<Option<JoinHandle<()>>>>,
}
pub enum ResourceStatus<T> {
    /// The resource is ready, you can use it
    Ready(T),
    /// The task to get the resource is currently running
    Pending,
    /// There is no task to get the resource. Use AsyncResource::set to set one.
    NotInitialized,
}

impl<T: 'static> AsyncResource<T> {
    /// Set a new task to initialize the resource, turning ResourceStatus to Pending until the resource is ready.
    /// If the task was already Pending, it cancels the previous task.
    /// Note: the task is spawned locally (tokio::task::spawn_local)
    pub fn set<Fut>(&self, f: Fut)
    where
        Fut: Future<Output = T> + 'static,
    {
        if let Some(handle) = self.handle.borrow().as_ref() {
            handle.abort();
        }
        let need_clear = self.need_clear.clone();
        let res = self.res.clone();
        let handle = self.handle.clone();
        let handle = tokio::task::spawn_local(async move {
            let t = f.await;
            *res.borrow_mut() = Some(t);
            *handle.borrow_mut() = None;
            // if we needed to clear before, we no longer need to
            need_clear.set(false);
        });
        *self.handle.borrow_mut() = Some(handle);
    }

    /// Cancel a pending task.
    pub fn cancel(&self) {
        if let Some(handle) = self.handle.borrow().as_ref() {
            handle.abort();
        }
        *self.handle.borrow_mut() = None;
    }

    pub fn get(&self) -> ResourceStatus<Ref<'_, T>> {
        if self.need_clear.get() {
            self.res.take();
            self.need_clear.set(false);
        }
        if self.handle.borrow().is_some() {
            return ResourceStatus::Pending;
        }

        match Ref::filter_map(self.res.borrow(), |opt| opt.as_ref()) {
            Ok(ref_t) => ResourceStatus::Ready(ref_t),
            Err(_) => ResourceStatus::NotInitialized,
        }
    }

    /// Set the resource manually instead of through a task.
    /// This cancels the current task.
    pub fn set_resource(&self, t: T) {
        self.cancel();
        *self.res.borrow_mut() = Some(t);
    }

    /// Clear the Resource if AsyncStatus::Ready. Otherwise it does nothing.
    pub fn clear(&self) {
        // we do this to allow clearing awhile holding a borrow to the resource via AsyncResource::get
        self.need_clear.set(true);
    }
}

impl<T> Default for AsyncResource<T> {
    fn default() -> Self {
        Self {
            res: Rc::new(RefCell::new(None)),
            need_clear: Rc::new(Cell::new(true)),
            handle: Rc::new(RefCell::new(None)),
        }
    }
}
