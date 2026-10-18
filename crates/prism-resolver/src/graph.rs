//! Import-directed dependency graph, Tarjan's SCC cycle grouping, and topological ordering.
//!
//! Implements the "Import-Directed Compilation Isolation" and "Dependency Graph and SCC Cycle
//! Decomposition" requirements of the `package-resolution` spec, and upholds the single-file
//! compilation unit and zero-marker invariants.

use crate::error::ResolveError;
use crate::import::resolve_import_with;
use crate::root::{absolute_path, discover_root_with, DirectoryListings};
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
        Self::build(entry_files, &mut SourceStore::new())
    }

    /// [`DependencyGraph::from_entry`], reading every file through `store` (D9 of
    /// `hot-reload`): its overlay first, then the content it holds for a file not invalidated
    /// since, then the disk. Only content that changed since the store last read it is parsed
    /// again, so a watch session that rebuilds the graph after an edit parses the edited files
    /// alone.
    pub fn from_entry_with(
        entry_file: &Path,
        store: &mut SourceStore,
    ) -> Result<Self, ResolveError> {
        Self::build(&[entry_file], store)
    }

    fn build(
        entry_files: &[impl AsRef<Path>],
        store: &mut SourceStore,
    ) -> Result<Self, ResolveError> {
        if entry_files.is_empty() {
            return Err(ResolveError::Io {
                path: PathBuf::new(),
                message: "No entry files provided to build dependency graph".to_string(),
            });
        }

        let first_entry = absolute_path(entry_files[0].as_ref());
        let (_, first_ast) = store.read(&first_entry)?;
        // Every directory the resolution looks into is read once (compile speed).
        let mut listings = DirectoryListings::with_virtual_files(store.overlay.keys().cloned());
        let authoritative_root =
            discover_root_with(&first_entry, first_ast.package.as_ref(), &mut listings)?;

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

            let (content, source_file) = store.read(&current_path)?;

            let current_root =
                discover_root_with(&current_path, source_file.package.as_ref(), &mut listings)?;

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
                let resolved_target = resolve_import_with(
                    &current_path,
                    import_decl,
                    &authoritative_root,
                    &mut listings,
                )?;
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

/// The source text and syntax tree of every file a watch session read, so that rebuilding its
/// dependency graph after an edit parses only the files whose content changed (D9 and D16 of
/// `hot-reload`). An overlay maps paths to content that replaces what is on disk: an edit
/// script's virtual edits, which leave every file on disk unchanged. Paths are absolute.
#[derive(Debug, Default)]
pub struct SourceStore {
    entries: HashMap<PathBuf, StoredSource>,
    overlay: HashMap<PathBuf, String>,
    /// Paths to read again from the overlay or the disk.
    stale: HashSet<PathBuf>,
    /// The number of files parsed through the store.
    parses: usize,
}

#[derive(Debug)]
struct StoredSource {
    content: String,
    file: SourceFile,
}

impl SourceStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes `path` read as `content` from now on, whatever the disk holds.
    pub fn set_overlay(&mut self, path: PathBuf, content: String) {
        self.stale.insert(path.clone());
        self.overlay.insert(path, content);
    }

    /// Marks `paths` to be read again at the next build.
    pub fn invalidate(&mut self, paths: &[PathBuf]) {
        self.stale.extend(paths.iter().cloned());
    }

    /// The number of files parsed through the store so far.
    pub fn parses(&self) -> usize {
        self.parses
    }

    /// Every path the store holds parsed content for, in no particular order.
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.entries.keys().map(PathBuf::as_path)
    }

    /// The content the store last read for `path`.
    pub fn content(&self, path: &Path) -> Option<&str> {
        self.entries.get(path).map(|entry| entry.content.as_str())
    }

    /// What `path` reads as now: its overlay content, else the disk's.
    pub fn current_content(&self, path: &Path) -> std::io::Result<String> {
        match self.overlay.get(path) {
            Some(content) => Ok(content.clone()),
            None => std::fs::read_to_string(path),
        }
    }

    /// Whether `path` reads differently now than when the store last read it: a file it never
    /// read, or one that cannot be read any more, counts as changed.
    pub fn has_changed(&self, path: &Path) -> bool {
        match (self.current_content(path), self.content(path)) {
            (Ok(now), Some(before)) => now != before,
            _ => true,
        }
    }

    /// The content and syntax tree of `path`: kept when the content is the one last read,
    /// parsed otherwise.
    fn read(&mut self, path: &Path) -> Result<(String, SourceFile), ResolveError> {
        let is_stale = self.stale.remove(path);
        if !is_stale {
            if let Some(entry) = self.entries.get(path) {
                return Ok((entry.content.clone(), entry.file.clone()));
            }
        }
        let content = self.current_content(path).map_err(|e| ResolveError::Io {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
        if let Some(entry) = self.entries.get(path) {
            if entry.content == content {
                return Ok((content, entry.file.clone()));
            }
        }
        self.parses += 1;
        let file = prism_parser::parse_source(&content).map_err(|e| {
            // A file that does not parse is read again next time, and its last good content is
            // forgotten: whatever it reads as next, even that content again, is a change.
            self.stale.insert(path.to_path_buf());
            self.entries.remove(path);
            ResolveError::Parse {
                path: path.to_path_buf(),
                error: e,
            }
        })?;
        self.entries.insert(
            path.to_path_buf(),
            StoredSource {
                content: content.clone(),
                file: file.clone(),
            },
        );
        Ok((content, file))
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
