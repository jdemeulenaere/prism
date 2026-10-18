//! Groups, the records of the slot table (D5 of `compose-runtime`, implementing
//! `compose-architecture` D6): a generational arena tree in which each group holds its key, its
//! kind, its parent, its ordered children and slots, its node for node groups and its restart
//! function for restart groups.

use crate::applier::NodeHandle;
use crate::handle::Handle;
use crate::slot::Slot;
use prism_runtime::FunctionValue;

/// What a group is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum GroupKind {
    /// A restart scope, which keeps a restart function and recomposes alone.
    Restart = 0,
    /// A group matched by its key alone.
    Replaceable = 1,
    /// A group matched by its key and its data key, so it keeps its state when siblings move.
    Movable = 2,
    /// A group that owns a node of the applier.
    Node = 3,
}

impl GroupKind {
    /// The word of the kind in dumps.
    pub fn name(self) -> &'static str {
        match self {
            GroupKind::Restart => "restart",
            GroupKind::Replaceable => "replaceable",
            GroupKind::Movable => "movable",
            GroupKind::Node => "node",
        }
    }
}

/// One group of the slot table.
#[derive(Debug)]
pub struct Group {
    pub key: i64,
    /// The data key of a movable group; 0 otherwise.
    pub data_key: i64,
    pub kind: GroupKind,
    pub composition: Handle,
    pub parent: Option<Handle>,
    pub children: Vec<Handle>,
    pub slots: Vec<Slot>,
    pub index_in_parent: u32,
    /// The node of a node group, of `node_kind`, and the node it was last placed into.
    pub node: Option<NodeHandle>,
    pub node_kind: i64,
    pub parent_node: Option<NodeHandle>,
    /// The last node, in group order, that this group's subtree places into its enclosing
    /// node: its own node for a node group.
    pub last_node: Option<NodeHandle>,
    /// The restart function of a restart group, or the content of a composition's root.
    pub restart: Option<FunctionValue>,
    /// Whether the scope is invalid and queued for the next pass.
    pub invalid: bool,
    /// Whether the scope was invalidated during the current pass, for the next one.
    pub pending: bool,
    /// Whether the group was removed in the current pass and waits for its release.
    pub removed: bool,
    /// The pass that last visited the group, and the pass that created it.
    pub visited: u64,
    pub created: u64,
}

impl Group {
    pub fn new(kind: GroupKind, key: i64, data_key: i64, composition: Handle) -> Group {
        Group {
            key,
            data_key,
            kind,
            composition,
            parent: None,
            children: Vec::new(),
            slots: Vec::new(),
            index_in_parent: 0,
            node: None,
            node_kind: 0,
            parent_node: None,
            last_node: None,
            restart: None,
            invalid: false,
            pending: false,
            removed: false,
            visited: 0,
            created: 0,
        }
    }

    /// Whether a start of `kind`, `key` and `data_key` matches the group.
    pub fn matches(&self, kind: GroupKind, key: i64, data_key: i64) -> bool {
        self.kind == kind
            && self.key == key
            && (kind != GroupKind::Movable || self.data_key == data_key)
    }
}
