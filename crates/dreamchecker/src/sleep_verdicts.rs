//! Which procs can park the proc that called them.
//!
//! When a proc sleeps, BYOND suspends it and every caller above it that still
//! waits for it. A JIT that compiles a caller has no interpreter frame for BYOND
//! to suspend, so it needs this answer for every proc before compiling it, not
//! just for the ones marked `SpacemanDMM_should_not_sleep`.
//!
//! Differences from the `must_not_sleep` lint, all on purpose:
//! - `SpacemanDMM_allowed_to_sleep` is ignored. It silences the lint, BYOND
//!   still suspends the caller.
//! - A call the analysis could not tie to a proc counts as sleeping. `call()()`
//!   does not: its targets are mostly signal handlers and callbacks, which
//!   codebases already require to not sleep, so one that does is their bug.
//! - Override dispatch is followed transitively, not one level deep.
//!
//! Not a proof: declared types are trusted, and DM does not enforce them.

use std::collections::BTreeMap;

use dm::objtree::{ProcRef, TypeRef};
use foldhash::HashMap;

use crate::{AnalyzeObjectTree, SleepAnalysisVersion};

impl<'o> AnalyzeObjectTree<'o> {
    /// Every non-builtin proc, with whether calling it can park the caller.
    ///
    /// A proc's own `set waitfor = 0` does not clear its verdict: a compiled
    /// frame has nowhere for BYOND to read that from. Only a callee's does.
    ///
    /// `unresolved_calls_sleep` false assumes every call the analysis could
    /// not tie to a proc does not sleep.
    pub fn sleep_verdicts(&self, unresolved_calls_sleep: bool) -> Vec<(ProcRef<'o>, bool)> {
        let mut roots = Vec::new();
        self.objtree.root().recurse(&mut |ty| {
            roots.extend(ty.iter_self_procs().filter(|proc| !proc.is_builtin()));
        });

        let mut graph = CallGraph::default();
        let root_nodes: Vec<usize> = roots
            .iter()
            .map(|&proc| {
                graph.intern(Visit {
                    proc,
                    receiver: proc.ty(),
                    is_exact: true,
                    receiver_is_stable: true,
                })
            })
            .collect();

        let mut next = 0;
        while next < graph.visits.len() {
            let from = next;
            let visit = graph.visits[from];
            next += 1;
            for target in self.visit_targets(visit) {
                // A callee with waitfor = 0 stops BYOND's walk up the callers,
                // so nothing below it can park anything above it.
                if !self.waitfor_procs.contains(&target.proc) {
                    let to = graph.intern(target);
                    graph.callers[to].push(from);
                }
            }
        }

        let mut parks = vec![false; graph.visits.len()];
        let mut queue: Vec<usize> = (0..graph.visits.len())
            .filter(|&index| {
                let proc = graph.visits[index].proc;
                self.sleeping_procs.violators.contains_key(&proc)
                    || (unresolved_calls_sleep && self.unresolved_calls.contains(&proc))
            })
            .collect();
        for &index in &queue {
            parks[index] = true;
        }
        while let Some(index) = queue.pop() {
            for &caller in &graph.callers[index] {
                if !parks[caller] {
                    parks[caller] = true;
                    queue.push(caller);
                }
            }
        }

        roots
            .into_iter()
            .zip(root_nodes)
            .map(|(proc, node)| (proc, parks[node]))
            .collect()
    }

    /// Where running `visit` can go next: the overrides it may dispatch to,
    /// then the procs its body calls outside `spawn`. The receiver scoping is
    /// the same as `check_proc_call_tree_v2_v3`; version 1 has no receiver
    /// scoping and follows every override.
    fn visit_targets(&self, visit: Visit<'o>) -> Vec<Visit<'o>> {
        let mut targets = Vec::new();
        let receiver_provenance = self.sleep_analysis_version.tracks_receiver_provenance();
        if !visit.is_exact
            && (!receiver_provenance || visit.receiver_is_stable)
            && visit.proc.ty() != self.objtree.root()
        {
            let mut push_override = |child: ProcRef<'o>| {
                targets.push(Visit {
                    proc: child,
                    receiver: child.ty(),
                    is_exact: true,
                    receiver_is_stable: true,
                })
            };
            match self.sleep_analysis_version {
                SleepAnalysisVersion::CallTree => visit.proc.recurse_children(&mut push_override),
                SleepAnalysisVersion::DynamicDispatch
                | SleepAnalysisVersion::ReceiverProvenance => visit
                    .proc
                    .recurse_children_within(visit.receiver, &mut push_override),
            }
        }

        let edges = self.call_tree.get(&visit.proc).into_iter().flatten();
        for edge in edges.filter(|edge| !edge.new_context) {
            let receiver = match self.sleep_analysis_version {
                SleepAnalysisVersion::CallTree => edge.proc.ty(),
                SleepAnalysisVersion::DynamicDispatch
                | SleepAnalysisVersion::ReceiverProvenance => {
                    if edge.inherit_receiver {
                        visit.receiver
                    } else {
                        edge.src
                    }
                },
            };
            targets.push(Visit {
                proc: edge.proc,
                receiver,
                is_exact: edge.is_exact,
                receiver_is_stable: !receiver_provenance
                    || (edge.inherit_receiver && visit.receiver_is_stable),
            });
        }
        targets
    }
}

/// One step of the linter's walk: a proc, the static type of the object it
/// runs on, and whether that object is known to be the root proc's own.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Visit<'o> {
    proc: ProcRef<'o>,
    receiver: TypeRef<'o>,
    is_exact: bool,
    receiver_is_stable: bool,
}

#[derive(Default)]
struct CallGraph<'o> {
    visits: Vec<Visit<'o>>,
    index: HashMap<Visit<'o>, usize>,
    callers: Vec<Vec<usize>>,
}

impl<'o> CallGraph<'o> {
    fn intern(&mut self, visit: Visit<'o>) -> usize {
        if let Some(&index) = self.index.get(&visit) {
            return index;
        }
        self.visits.push(visit);
        self.callers.push(Vec::new());
        self.index.insert(visit, self.visits.len() - 1);
        self.visits.len() - 1
    }
}

/// Paths of the procs that can never park their caller, spelled the way the
/// `.dmb` spells them. Copies that share a spelling (two overrides on one
/// type) are listed only if every copy is safe.
pub fn allowlist(analyzer: &AnalyzeObjectTree, unresolved_calls_sleep: bool) -> Vec<String> {
    let mut safe_by_path: BTreeMap<String, bool> = BTreeMap::new();
    for (proc, parks) in analyzer.sleep_verdicts(unresolved_calls_sleep) {
        *safe_by_path.entry(dmb_path(proc)).or_insert(true) &= !parks;
    }
    safe_by_path
        .into_iter()
        .filter_map(|(path, safe)| safe.then_some(path))
        .collect()
}

/// The `.dmb` keeps `/proc/` or `/verb/` only on the copy that declares the
/// proc, so an override is `/turf/open/process_cell` and a redefinition on the
/// declaring type is `/turf/ChangeTurf`. The object tree always keeps the
/// declaring copy first.
fn dmb_path(proc: ProcRef) -> String {
    let ty = proc.ty();
    match ty
        .get()
        .procs
        .get(proc.name())
        .and_then(|type_proc| type_proc.declaration.as_ref())
    {
        Some(declaration) if proc.index() == 0 => {
            format!("{}/{}/{}", ty.path, declaration.kind, proc.name())
        },
        _ => format!("{}/{}", ty.path, proc.name()),
    }
}
