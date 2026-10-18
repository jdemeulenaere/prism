//! Import-directed dependency graph, Tarjan's SCC cycle grouping, and topological ordering.
//!
//! Implements the "Import-Directed Compilation Isolation" and "Dependency Graph and SCC Cycle
//! Decomposition" requirements of the `package-resolution` spec, and upholds the single-file
//! compilation unit and zero-marker invariants.

use crate::error::ResolveError;
use crate::import::resolve_import;
use crate::root::{absolute_path, discover_root};
use prism_ast::SourceFile;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// A compiled compilation unit in the dependency graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceNode {
    /// Canonical path to this source file.
    pub path: PathBuf,
    /// Discovered workspace root for this compilation graph.
    pub workspace_root: PathBuf,
    /// Declared package segments (e.g. `["foo", "bar"]` or empty for root files).
    pub package: Vec<String>,
    /// The source text that was parsed, so that later stages (the build cache's content key)
    /// see exactly what was checked even if the file changes on disk meanwhile.
    pub source: String,
    /// Parsed AST of the source file.
    pub source_file: SourceFile,
    /// Resolved target paths of direct imports.
    pub direct_imports: Vec<PathBuf>,
}

/// A Strongly Connected Component (SCC) cluster in the dependency graph.
///
/// Files in the same cluster are mutually recursive and must have their Phase 1 ABIs
/// extracted together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SccCluster {
    /// Source file paths in this SCC cluster.
    pub files: Vec<PathBuf>,
}

/// An import-directed dependency graph of source files and their SCC compilation order.
#[derive(Debug, Clone)]
pub struct DependencyGraph {
    /// Authoritative workspace root shared across all files in this graph.
    pub workspace_root: PathBuf,
    /// All reachable source nodes indexed by their path.
    pub nodes: HashMap<PathBuf, SourceNode>,
    /// Strongly connected components ordered dependency-first (topological compilation schedule).
    pub sccs: Vec<SccCluster>,
}

impl DependencyGraph {
    /// Constructs an import-directed dependency graph starting from a single entry file.
    pub fn from_entry(entry_file: &Path) -> Result<Self, ResolveError> {
        Self::from_entries(&[entry_file])
    }

    /// Constructs an import-directed dependency graph starting from one or more entry files.
    ///
    /// Relative entry paths are made absolute against the current directory first, so the root
    /// is discovered from the file's real location; every path in the graph is absolute.
    pub fn from_entries(entry_files: &[impl AsRef<Path>]) -> Result<Self, ResolveError> {
        if entry_files.is_empty() {
            return Err(ResolveError::Io {
                path: PathBuf::new(),
                message: "No entry files provided to build dependency graph".to_string(),
            });
        }

        let first_entry = absolute_path(entry_files[0].as_ref());
        let first_content =
            std::fs::read_to_string(&first_entry).map_err(|e| ResolveError::Io {
                path: first_entry.clone(),
                message: e.to_string(),
            })?;
        let first_ast =
            prism_parser::parse_source(&first_content).map_err(|e| ResolveError::Parse {
                path: first_entry.clone(),
                error: e,
            })?;
        let authoritative_root = discover_root(&first_entry, first_ast.package.as_ref())?;

        let mut worklist: VecDeque<PathBuf> = VecDeque::new();
        let mut visited: HashSet<PathBuf> = HashSet::new();
        let mut nodes: HashMap<PathBuf, SourceNode> = HashMap::new();

        for entry in entry_files {
            worklist.push_back(absolute_path(entry.as_ref()));
        }

        while let Some(current_path) = worklist.pop_front() {
            if visited.contains(&current_path) {
                continue;
            }
            visited.insert(current_path.clone());

            let content = std::fs::read_to_string(&current_path).map_err(|e| ResolveError::Io {
                path: current_path.clone(),
                message: e.to_string(),
            })?;
            let source_file =
                prism_parser::parse_source(&content).map_err(|e| ResolveError::Parse {
                    path: current_path.clone(),
                    error: e,
                })?;

            let current_root = discover_root(&current_path, source_file.package.as_ref())?;

            // Root consistency check (E1004 InconsistentWorkspaceRoot)
            let roots_differ = if current_root == authoritative_root {
                false
            } else if let (Ok(c1), Ok(c2)) = (
                current_root.canonicalize(),
                authoritative_root.canonicalize(),
            ) {
                c1 != c2
            } else {
                true
            };

            if roots_differ {
                return Err(ResolveError::InconsistentWorkspaceRoot {
                    file: current_path,
                    file_root: current_root,
                    workspace_root: authoritative_root,
                });
            }

            let mut direct_imports = Vec::new();
            for import_decl in &source_file.imports {
                let resolved_target =
                    resolve_import(&current_path, import_decl, &authoritative_root)?;
                if !direct_imports.contains(&resolved_target) {
                    direct_imports.push(resolved_target.clone());
                }
                if !visited.contains(&resolved_target) {
                    worklist.push_back(resolved_target);
                }
            }

            let package = source_file
                .package
                .as_ref()
                .map(|p| p.path.clone())
                .unwrap_or_default();

            let node = SourceNode {
                path: current_path.clone(),
                workspace_root: authoritative_root.clone(),
                package,
                source: content,
                source_file,
                direct_imports,
            };

            nodes.insert(current_path, node);
        }

        let sccs = compute_sccs(&nodes);

        Ok(Self {
            workspace_root: authoritative_root,
            nodes,
            sccs,
        })
    }

    /// Returns a reference to the source node at the given path, if present.
    pub fn get_node(&self, path: &Path) -> Option<&SourceNode> {
        self.nodes.get(path)
    }

    /// Returns the topologically sorted Strongly Connected Components (dependency-first order).
    pub fn sccs(&self) -> &[SccCluster] {
        &self.sccs
    }

    /// Alias for `sccs()`, returning the dependency-first topological compilation schedule.
    pub fn topological_order(&self) -> &[SccCluster] {
        &self.sccs
    }

    /// Returns true if the dependency graph contains any cyclic SCC clusters or self-loops.
    pub fn is_cyclic(&self) -> bool {
        self.sccs.iter().any(|c| {
            if c.files.len() > 1 {
                true
            } else if let Some(single_file) = c.files.first() {
                if let Some(node) = self.nodes.get(single_file) {
                    node.direct_imports.contains(single_file)
                } else {
                    false
                }
            } else {
                false
            }
        })
    }

    /// Returns the total number of files in the dependency graph.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Returns true if the dependency graph contains no files.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// Tarjan's strongly connected components algorithm producing a dependency-first topological sort.
fn compute_sccs(nodes: &HashMap<PathBuf, SourceNode>) -> Vec<SccCluster> {
    let mut file_list: Vec<PathBuf> = nodes.keys().cloned().collect();
    file_list.sort(); // Deterministic file ordering

    let file_to_id: HashMap<PathBuf, usize> = file_list
        .iter()
        .enumerate()
        .map(|(idx, path)| (path.clone(), idx))
        .collect();

    let n = file_list.len();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];

    for (path, node) in nodes {
        if let Some(&u) = file_to_id.get(path) {
            for imp in &node.direct_imports {
                if let Some(&v) = file_to_id.get(imp) {
                    adj[u].push(v);
                }
            }
            adj[u].sort();
            adj[u].dedup();
        }
    }

    let mut state = TarjanState::new(&adj);

    for u in 0..n {
        if !state.indices.contains_key(&u) {
            state.strongconnect(u);
        }
    }

    let mut clusters = Vec::new();
    for raw in state.sccs {
        let mut files: Vec<PathBuf> = raw.into_iter().map(|id| file_list[id].clone()).collect();
        files.sort();
        clusters.push(SccCluster { files });
    }

    clusters
}

struct TarjanState<'a> {
    current_index: usize,
    stack: Vec<usize>,
    on_stack: HashSet<usize>,
    indices: HashMap<usize, usize>,
    lowlink: HashMap<usize, usize>,
    adj: &'a [Vec<usize>],
    sccs: Vec<Vec<usize>>,
}

impl<'a> TarjanState<'a> {
    fn new(adj: &'a [Vec<usize>]) -> Self {
        Self {
            current_index: 0,
            stack: Vec::new(),
            on_stack: HashSet::new(),
            indices: HashMap::new(),
            lowlink: HashMap::new(),
            adj,
            sccs: Vec::new(),
        }
    }

    fn strongconnect(&mut self, u: usize) {
        self.indices.insert(u, self.current_index);
        self.lowlink.insert(u, self.current_index);
        self.current_index += 1;
        self.stack.push(u);
        self.on_stack.insert(u);

        for &v in &self.adj[u] {
            if !self.indices.contains_key(&v) {
                self.strongconnect(v);
                let v_low = self.lowlink[&v];
                let u_low = self.lowlink.get_mut(&u).unwrap();
                *u_low = (*u_low).min(v_low);
            } else if self.on_stack.contains(&v) {
                let v_idx = self.indices[&v];
                let u_low = self.lowlink.get_mut(&u).unwrap();
                *u_low = (*u_low).min(v_idx);
            }
        }

        if self.lowlink[&u] == self.indices[&u] {
            let mut scc = Vec::new();
            while let Some(w) = self.stack.pop() {
                self.on_stack.remove(&w);
                scc.push(w);
                if w == u {
                    break;
                }
            }
            self.sccs.push(scc);
        }
    }
}
