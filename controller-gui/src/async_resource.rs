#[cfg(target_arch = "wasm32")]
use futures::future::{AbortHandle, Abortable};
use std::{
    cell::{Cell, Ref, RefCell},
    rc::Rc,
};

// hack because we don't need Send in wasm, it creates problems
#[cfg(not(target_arch = "wasm32"))]
pub trait Bounds: Send + 'static {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: ?Sized + Send + 'static> Bounds for T {}

#[cfg(target_arch = "wasm32")]
pub trait Bounds: 'static {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized + 'static> Bounds for T {}

/// A resource which can be acquired asynchornously (wrapper around poll_promise)
pub struct AsyncResource<T: Bounds> {
    res: Rc<RefCell<Option<T>>>,
    need_clear: Cell<bool>,
    #[cfg(not(target_arch = "wasm32"))]
    handle: Rc<RefCell<Option<poll_promise::Promise<T>>>>,
    // poll_promise needs T: Send for some reason.
    #[cfg(target_arch = "wasm32")]
    handle: Rc<RefCell<Option<AbortHandle>>>,
}
pub enum ResourceStatus<T> {
    /// The resource is ready, you can use it
    Ready(T),
    /// The task to get the resource is currently running
    Pending,
    /// There is no task to get the resource. Use AsyncResource::set to set one.
    NotInitialized,
}

impl<T: Bounds> AsyncResource<T> {
    /// Set a new task to initialize the resource, turning ResourceStatus to Pending until the resource is ready.
    /// If the task was already Pending, it cancels the previous task.
    pub fn set<Fut>(&self, f: Fut)
    where
        Fut: Future<Output = T> + Bounds,
    {
        if let Some(task) = self.handle.take() {
            task.abort();
        }
        #[cfg(not(target_arch = "wasm32"))]
        let new_handle = poll_promise::Promise::spawn_async(f);
        #[cfg(target_arch = "wasm32")]
        let new_handle = {
            let res = self.res.clone();
            let handle = self.handle.clone();
            let future = async move {
                let t = f.await;
                *res.borrow_mut() = Some(t);
                *handle.borrow_mut() = None;
            };

            let (new_handle, abort_reg) = AbortHandle::new_pair();
            let abortable = Abortable::new(future, abort_reg);
            wasm_bindgen_futures::spawn_local(async move {
                let _ = abortable.await;
            });
            new_handle
        };
        *self.handle.borrow_mut() = Some(new_handle);
    }

    /// Cancel a pending task.
    pub fn cancel(&self) {
        if let Some(handle) = self.handle.borrow_mut().take() {
            handle.abort();
        }
        *self.handle.borrow_mut() = None;
    }

    pub fn get(&self) -> ResourceStatus<Ref<'_, T>> {
        if self.need_clear.take() {
            self.res.take();
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let maybe_promise = self.handle.borrow_mut().take();
            if let Some(promise) = maybe_promise {
                match promise.try_take() {
                    Ok(val) => {
                        *self.res.borrow_mut() = Some(val);
                    }
                    Err(promise) => {
                        *self.handle.borrow_mut() = Some(promise);
                        return ResourceStatus::Pending;
                    }
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            if self.handle.borrow().is_some() {
                return ResourceStatus::Pending;
            }
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

impl<T: Bounds> Default for AsyncResource<T> {
    fn default() -> Self {
        Self {
            res: Rc::new(RefCell::new(None)),
            need_clear: Cell::new(false),
            handle: Rc::new(RefCell::new(None)),
        }
    }
}
