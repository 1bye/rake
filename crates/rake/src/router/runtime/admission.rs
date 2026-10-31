//! Runtime evaluation for typed route admission.

use std::rc::Rc;
use tracing::Instrument;

use super::{Action, Core, Stop};
use crate::router::{Admission, AdmissionContext, OperationId, Route};

/// Runs typed admission before an action is planned into an entry.
pub(crate) async fn evaluate<R: Route>(
    core: &Rc<Core<R>>,
    id: OperationId,
    action: &Action<R>,
    origin_window: crate::router::WindowId,
) -> Result<Action<R>, Stop> {
    let mut candidate = action.clone();
    let mut redirects = 0;

    'restart: loop {
        candidate.check(core)?;

        let Some(destination) = candidate.destination() else {
            return Ok(candidate);
        };

        destination
            .to_uri()
            .map_err(crate::router::OperationError::from)?;

        let callbacks = core.admissions.borrow().clone();

        for callback in callbacks {
            let context = AdmissionContext {
                operation_id: id,
                window_id: origin_window,
                target_tab: candidate.target_tab(),
                destination: destination.clone(),
                services: crate::control::RuntimeClient::new(
                    super::Router(core.clone()),
                    core.control.clone(),
                )
                .services(),
            };

            let result = callback(context)
                .instrument(tracing::info_span!("router.admission", operation_id = %id))
                .await;

            candidate.check(core)?;

            match result {
                Admission::Allow => {}
                Admission::Deny(reason) => {
                    return Err(Stop::Blocked(crate::router::BlockReason::Guard(reason)));
                }
                Admission::Redirect(destination) => {
                    redirects += 1;

                    if redirects > core.config.max_admission_redirects {
                        return Err(crate::router::OperationError::AdmissionRedirectLoop {
                            limit: core.config.max_admission_redirects,
                        }
                        .into());
                    }

                    candidate = candidate.with_destination(destination);
                    core.navigation_redirect(id, &candidate, origin_window);
                    continue 'restart;
                }
            }
        }

        return Ok(candidate);
    }
}
