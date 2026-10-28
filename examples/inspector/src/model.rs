//! Runtime/capture-scoped selection. A fresh capture never silently retargets a selection.

use crate::Instance;
use rake::inspection::{Element, Snapshot};
use rake::router::RuntimeId;

/// Owned connection results, published through Rake's managed activity lifecycle.
pub enum Update {
    /// Discovery results for the current connection epoch.
    Instances(u64, Result<Vec<Instance>, String>),
    /// Picker observations, deduplicated by sequence and scoped to the exact capture.
    Pick(u64, RuntimeId, Result<rake::inspection::Pick, String>),
    /// Exact runtime tree response.
    Tree(u64, RuntimeId, Result<Snapshot, String>),
    /// Exact capture-scoped element response.
    Element(u64, RuntimeId, String, Box<Result<Element, String>>),
}

/// Authoritative frontend state; contains owned remote metadata, never target UI entities.
#[derive(Default)]
pub struct Inspector {
    /// Latest advertisements from the broker.
    pub instances: Vec<Instance>,
    /// Concrete target runtime, independent of the Inspector's own runtime.
    pub runtime: Option<RuntimeId>,
    /// Latest bounded capture.
    pub tree: Option<Snapshot>,
    /// Explicitly selected opaque element ID.
    pub selected: Option<String>,
    /// Frozen selected capture metadata.
    pub detail: Option<Element>,
    /// Last transport or lookup error.
    pub error: Option<String>,
    /// An exact element lookup failure, kept independently of successful tree polling.
    pub lookup_error: Option<String>,
    /// Search text applied to supplied metadata.
    pub filter: String,
    /// Pause tree updates; explicit refresh remains available.
    pub paused: bool,
    /// Whether this frontend is renewing the target's input-capture lease.
    pub picking: bool,
    pub(crate) mode_pending: Option<bool>,
    pub(crate) hover: Option<(String, String)>,
    pick_sequence: u64,
    pending_pick: Option<rake::inspection::Pick>,
    /// User disconnected from observation.
    pub disconnected: bool,
    /// Invalidates responses issued before connection/target changes.
    pub epoch: u64,
    pub(crate) detail_pending: bool,
    pub(crate) refresh: bool,
    selector: Option<String>,
    application: Option<String>,
}

impl Inspector {
    /// Starts with an optional runtime/application selector from the launch command.
    pub fn new(selector: Option<String>) -> Self {
        Self {
            selector,
            refresh: true,
            ..Self::default()
        }
    }

    /// Explicit target change clears capture IDs and makes older replies inapplicable.
    pub fn select_runtime(&mut self, runtime: Option<RuntimeId>) {
        self.epoch += 1;
        self.runtime = runtime;
        self.application = self
            .instances
            .iter()
            .find(|instance| Some(instance.info.runtime_id) == runtime)
            .map(|instance| instance.application_id.clone());
        self.tree = None;
        self.picking = false;
        self.mode_pending = None;
        self.hover = None;
        self.pick_sequence = 0;
        self.pending_pick = None;
        self.selected = None;
        self.detail = None;
        self.detail_pending = false;
        self.error = None;
        self.lookup_error = None;
        self.refresh = true;
    }

    /// Selects an exact ID from the currently displayed capture.
    pub fn select_element(&mut self, id: &str) {
        if let Some(element) = self
            .tree
            .as_ref()
            .and_then(|tree| tree.elements.iter().find(|element| element.id == id))
        {
            // The tree already owns this exact capture's metadata. Keep it even
            // if the target redraws before the confirming lookup arrives.
            self.selected = Some(id.into());
            self.detail = Some(element.clone());
            self.lookup_error = None;
            self.detail_pending = true;
        }
    }

    fn apply_pick(&mut self) {
        if self.pending_pick.as_ref().is_some_and(|pick| {
            self.tree
                .as_ref()
                .is_some_and(|tree| tree.capture == pick.generation)
        }) && let Some(pick) = self.pending_pick.take()
            && let Some(id) = pick.picked
        {
            self.select_element(&id);
        }
    }

    /// Applies a publication only when its connection/runtime/selection is still current.
    pub fn apply(&mut self, update: Update) {
        match update {
            Update::Pick(epoch, runtime, result)
                if epoch == self.epoch && Some(runtime) == self.runtime && !self.disconnected =>
            {
                match result {
                    Ok(pick) => {
                        self.picking = pick.mode;
                        if pick.picked.is_some() && pick.pick_sequence > self.pick_sequence {
                            self.pick_sequence = pick.pick_sequence;
                            self.pending_pick = Some(pick);
                            self.refresh = true;
                            self.apply_pick();
                        }
                    }
                    Err(error) => {
                        self.picking = false;
                        self.error = Some(error);
                    }
                }
            }

            Update::Instances(epoch, result) if epoch == self.epoch => match result {
                Ok(instances) => {
                    if let Some(runtime) = self.runtime {
                        if !instances
                            .iter()
                            .any(|instance| instance.info.runtime_id == runtime)
                        {
                            let successors: Vec<_> = instances
                                .iter()
                                .filter(|instance| {
                                    instance.restored_from == Some(runtime)
                                        && self.application.as_ref()
                                            == Some(&instance.application_id)
                                })
                                .collect();

                            if successors.len() == 1 {
                                self.select_runtime(Some(successors[0].info.runtime_id));
                            } else {
                                self.error =
                                    Some("Target disconnected. Select a live runtime.".into());
                            }
                        }
                    } else if !self.disconnected {
                        let matches: Vec<_> = instances
                            .iter()
                            .filter(|instance| {
                                self.selector.as_ref().is_none_or(|selector| {
                                    selector == &instance.application_id
                                        || selector == &instance.info.runtime_id.to_string()
                                })
                            })
                            .collect();

                        if matches.len() == 1 {
                            self.select_runtime(Some(matches[0].info.runtime_id));
                        }
                    }

                    if let Some(instance) = instances
                        .iter()
                        .find(|instance| Some(instance.info.runtime_id) == self.runtime)
                    {
                        self.application = Some(instance.application_id.clone());
                    }

                    self.instances = instances;
                }
                Err(error) => self.error = Some(error),
            },
            Update::Tree(epoch, runtime, result)
                if epoch == self.epoch && Some(runtime) == self.runtime && !self.disconnected =>
            {
                match result {
                    Ok(tree) => {
                        if self.hover.as_ref().is_some_and(|(id, generation)| {
                            generation != &tree.capture
                                || !tree.elements.iter().any(|element| &element.id == id)
                        }) {
                            self.hover = None;
                        }
                        if let Some(current) = self
                            .selected
                            .as_ref()
                            .and_then(|id| tree.elements.iter().find(|element| &element.id == id))
                        {
                            self.detail = Some(current.clone());
                        }
                        self.tree = Some(tree);
                        self.error = None;
                        self.apply_pick();
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Update::Element(epoch, runtime, id, result)
                if epoch == self.epoch
                    && Some(runtime) == self.runtime
                    && self.selected.as_ref() == Some(&id)
                    && !self.disconnected =>
            {
                match *result {
                    Ok(element) if element.id == id => {
                        self.detail = Some(element);
                        self.lookup_error = None;
                    }
                    Ok(_) => {
                        self.lookup_error =
                            Some("Inspector returned a different element identity.".into())
                    }
                    Err(error) => self.lookup_error = Some(error),
                }
            }
            _ => {}
        }
    }

    /// Whether the selected detail belongs to a replaced capture.
    pub fn stale(&self) -> bool {
        self.selected.as_ref().is_some_and(|id| {
            self.tree
                .as_ref()
                .is_none_or(|tree| !tree.elements.iter().any(|element| &element.id == id))
        })
    }

    /// Displayed nodes matching semantic identity, type, opaque ID or source location.
    pub fn visible_elements(&self) -> Vec<&Element> {
        let query = self.filter.to_lowercase();

        self.tree
            .iter()
            .flat_map(|tree| &tree.elements)
            .filter(|element| {
                query.is_empty()
                    || format!(
                        "{} {} {} {}",
                        element.id,
                        element.kind,
                        element.semantic_id.as_deref().unwrap_or(""),
                        element
                            .source
                            .as_ref()
                            .map_or("", |source| source.file.as_str())
                    )
                    .to_lowercase()
                    .contains(&query)
            })
            .collect()
    }
}
