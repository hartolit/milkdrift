//! Slint presentation adapter over the frontend-neutral application runtime.

#![deny(unsafe_code)]

mod error;
mod paths;
mod presenter;

use std::cell::RefCell;
use std::rc::Rc;

use application_runtime::{ApplicationRuntime, ApplicationRuntimeConfiguration};

use slint::ComponentHandle;

pub use error::DesktopError;

#[allow(
    missing_docs,
    clippy::all,
    clippy::pedantic,
    clippy::nursery,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod generated {
    slint::include_modules!();
}

use generated::AppWindow;

/// Starts the native Slint frontend and performs bounded worker shutdown on exit.
///
/// # Errors
///
/// Returns an error if the application data path cannot be prepared, the runtime cannot start or
/// stop cleanly, or Slint cannot create or run the application window.
pub fn run() -> Result<(), DesktopError> {
    let configuration =
        ApplicationRuntimeConfiguration::desktop(paths::application_database_path()?);
    let mut runtime = ApplicationRuntime::start(configuration)?;
    let window = match AppWindow::new() {
        Ok(window) => window,
        Err(slint) => {
            return match runtime.shutdown() {
                Ok(()) => Err(DesktopError::Slint(slint)),
                Err(shutdown) => Err(DesktopError::SlintAndShutdown { slint, shutdown }),
            };
        }
    };
    window.set_repository(runtime.preferences().default_repository.clone().into());
    window.set_revision(runtime.preferences().default_revision.clone().into());

    let runtime = Rc::new(RefCell::new(runtime));
    let presenter = presenter::Presenter::connect(&window, &runtime);
    presenter.synchronize_controls(&window, &runtime.borrow());
    let timer = presenter.start_frame_timer(&window, Rc::clone(&runtime));
    let run_result = window.run();
    timer.stop();
    let shutdown_result = runtime.borrow_mut().shutdown();
    drop(window);

    match (run_result, shutdown_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(slint), Ok(())) => Err(DesktopError::Slint(slint)),
        (Ok(()), Err(shutdown)) => Err(DesktopError::Application(shutdown)),
        (Err(slint), Err(shutdown)) => Err(DesktopError::SlintAndShutdown { slint, shutdown }),
    }
}
