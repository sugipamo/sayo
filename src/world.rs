use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::error::{Error, Result};

/// A `/`-separated path inside a World snapshot.
///
/// World paths are deliberately independent of host filesystem paths so the
/// same evaluator can address an `FsWorld` and a browser-owned `MemoryWorld`.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorldPath(String);

impl WorldPath {
    pub fn root() -> Self {
        Self(String::new())
    }

    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if path.is_absolute() {
            return Err(Error::World(format!(
                "World path must be relative: {}",
                path.display()
            )));
        }
        let mut parts = Vec::new();
        for component in path.components() {
            match component {
                Component::Normal(part) => {
                    let part = part.to_str().ok_or_else(|| {
                        Error::World(format!("World path is not valid UTF-8: {}", path.display()))
                    })?;
                    if part.is_empty() {
                        return Err(Error::World(
                            "World path contains an empty component".into(),
                        ));
                    }
                    parts.push(part);
                }
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(Error::World(format!(
                        "World path cannot escape its root: {}",
                        path.display()
                    )));
                }
            }
        }
        Ok(Self(parts.join("/")))
    }

    pub fn join(&self, child: impl AsRef<Path>) -> Result<Self> {
        let child = Self::new(child)?;
        if self.0.is_empty() {
            return Ok(child);
        }
        if child.0.is_empty() {
            return Ok(self.clone());
        }
        Ok(Self(format!("{}/{}", self.0, child.0)))
    }

    /// Resolve a relative source reference while allowing `..` to move within
    /// the virtual root. Escaping the snapshot and host-absolute paths are
    /// rejected by the host boundary.
    pub fn resolve(&self, reference: impl AsRef<Path>) -> Result<Self> {
        let reference = reference.as_ref();
        if reference.is_absolute() {
            return Err(Error::Source(format!(
                "absolute paths are unavailable in a World snapshot: {}",
                reference.display()
            )));
        }
        let mut parts = self
            .0
            .split('/')
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        for component in reference.components() {
            match component {
                Component::Normal(part) => {
                    let part = part.to_str().ok_or_else(|| {
                        Error::Source(format!(
                            "source path is not valid UTF-8: {}",
                            reference.display()
                        ))
                    })?;
                    parts.push(part.to_owned());
                }
                Component::CurDir => {}
                Component::ParentDir => {
                    if parts.pop().is_none() {
                        return Err(Error::Source(format!(
                            "source path escapes its World snapshot: {}",
                            reference.display()
                        )));
                    }
                }
                Component::RootDir | Component::Prefix(_) => unreachable!("absolute path checked"),
            }
        }
        Ok(Self(parts.join("/")))
    }

    pub fn parent(&self) -> Option<Self> {
        self.0
            .rsplit_once('/')
            .map(|(parent, _)| Self(parent.to_owned()))
            .or_else(|| (!self.0.is_empty()).then(Self::root))
    }

    pub fn file_name(&self) -> Option<&str> {
        (!self.0.is_empty())
            .then(|| self.0.rsplit('/').next())
            .flatten()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn top_component(&self) -> Option<&str> {
        self.0.split('/').next().filter(|part| !part.is_empty())
    }
}

/// Read-only access to one logically immutable World layer.
pub trait WorldView: Send + Sync {
    fn read(&self, path: &WorldPath) -> Result<Option<String>>;
    fn is_file(&self, path: &WorldPath) -> Result<bool>;
    fn is_dir(&self, path: &WorldPath) -> Result<bool>;
    fn directories(&self, path: &WorldPath) -> Result<Vec<WorldPath>>;
    fn files(&self, path: &WorldPath) -> Result<Vec<WorldPath>>;
}

/// Shared ownership of an immutable World view.
#[derive(Clone)]
pub struct WorldHandle(Arc<dyn WorldView>);

impl WorldHandle {
    pub fn new(world: impl WorldView + 'static) -> Self {
        Self(Arc::new(world))
    }

    pub fn root(&self) -> VirtualPath {
        VirtualPath {
            world: self.clone(),
            path: WorldPath::root(),
        }
    }

    pub fn view(&self) -> &dyn WorldView {
        self.0.as_ref()
    }

    fn same_world(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for WorldHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("WorldHandle")
            .field(&(Arc::as_ptr(&self.0) as *const () as usize))
            .finish()
    }
}

/// A path bound to a particular immutable World view. Unlike `PathBuf`, all
/// observations go through `WorldView`, so it works identically in native and
/// browser hosts.
#[derive(Clone, Debug)]
pub struct VirtualPath {
    world: WorldHandle,
    path: WorldPath,
}

impl VirtualPath {
    pub fn world_path(&self) -> &WorldPath {
        &self.path
    }

    pub fn join(&self, child: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            world: self.world.clone(),
            path: self.path.join(child)?,
        })
    }

    pub fn resolve(&self, reference: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            world: self.world.clone(),
            path: self.path.resolve(reference)?,
        })
    }

    pub fn parent(&self) -> Option<Self> {
        self.path.parent().map(|path| Self {
            world: self.world.clone(),
            path,
        })
    }

    pub fn file_name(&self) -> Option<&str> {
        self.path.file_name()
    }

    pub fn display(&self) -> &str {
        self.path.as_str()
    }

    pub fn read(&self) -> Result<Option<String>> {
        self.world.0.read(&self.path)
    }

    pub fn is_file(&self) -> Result<bool> {
        self.world.0.is_file(&self.path)
    }

    pub fn is_dir(&self) -> Result<bool> {
        self.world.0.is_dir(&self.path)
    }

    pub fn directories(&self) -> Result<Vec<Self>> {
        Ok(self
            .world
            .0
            .directories(&self.path)?
            .into_iter()
            .map(|path| Self {
                world: self.world.clone(),
                path,
            })
            .collect())
    }

    pub fn files(&self) -> Result<Vec<Self>> {
        Ok(self
            .world
            .0
            .files(&self.path)?
            .into_iter()
            .map(|path| Self {
                world: self.world.clone(),
                path,
            })
            .collect())
    }
}

impl PartialEq for VirtualPath {
    fn eq(&self, other: &Self) -> bool {
        self.world.same_world(&other.world) && self.path == other.path
    }
}

impl Eq for VirtualPath {}

/// Native-host adapter. The root is canonicalized once and all subsequent
/// accesses are constrained to validated relative `WorldPath`s.
#[derive(Clone, Debug)]
pub struct FsWorld {
    root: PathBuf,
}

impl FsWorld {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let requested = root.as_ref();
        let root = fs::canonicalize(requested).map_err(|error| {
            Error::World(format!(
                "cannot open World {}: {error}",
                requested.display()
            ))
        })?;
        if !root.is_dir() {
            return Err(Error::World(format!(
                "World does not exist: {}",
                root.display()
            )));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn resolve(&self, path: &WorldPath) -> PathBuf {
        path.as_str()
            .split('/')
            .filter(|part| !part.is_empty())
            .fold(self.root.clone(), |path, part| path.join(part))
    }
}

impl WorldView for FsWorld {
    fn read(&self, path: &WorldPath) -> Result<Option<String>> {
        let path = self.resolve(path);
        if !path.is_file() {
            return Ok(None);
        }
        Ok(Some(fs::read_to_string(path)?))
    }

    fn is_file(&self, path: &WorldPath) -> Result<bool> {
        Ok(self.resolve(path).is_file())
    }

    fn is_dir(&self, path: &WorldPath) -> Result<bool> {
        Ok(self.resolve(path).is_dir())
    }

    fn directories(&self, path: &WorldPath) -> Result<Vec<WorldPath>> {
        let resolved = self.resolve(path);
        if !resolved.is_dir() {
            return Ok(Vec::new());
        }
        let mut directories = fs::read_dir(resolved)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().is_dir())
            .map(|entry| path.join(entry.file_name()))
            .collect::<Result<Vec<_>>>()?;
        directories.sort();
        Ok(directories)
    }

    fn files(&self, path: &WorldPath) -> Result<Vec<WorldPath>> {
        let resolved = self.resolve(path);
        if !resolved.is_dir() {
            return Ok(Vec::new());
        }
        let mut files = fs::read_dir(resolved)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().is_file())
            .map(|entry| path.join(entry.file_name()))
            .collect::<Result<Vec<_>>>()?;
        files.sort();
        Ok(files)
    }
}

/// Browser-compatible World storage. Directories are implicit in file paths,
/// but explicit empty directories are retained because they carry Sayo
/// Relation meaning.
#[derive(Clone, Debug, Default)]
pub struct MemoryWorld {
    files: BTreeMap<WorldPath, String>,
    directories: BTreeSet<WorldPath>,
}

impl MemoryWorld {
    pub fn new() -> Self {
        let mut world = Self::default();
        world.directories.insert(WorldPath::root());
        world
    }

    pub fn from_files<I, P, S>(files: I) -> Result<Self>
    where
        I: IntoIterator<Item = (P, S)>,
        P: AsRef<Path>,
        S: Into<String>,
    {
        let mut world = Self::new();
        for (path, source) in files {
            world.insert_file(path, source)?;
        }
        Ok(world)
    }

    /// Copy a logically immutable view into browser-compatible owned memory.
    /// Explicit empty directories are preserved because an implementationless
    /// Relation directory is meaningful in Sayo.
    pub fn snapshot(source: &dyn WorldView) -> Result<Self> {
        let mut snapshot = Self::new();
        let mut pending = vec![WorldPath::root()];
        while let Some(directory) = pending.pop() {
            if directory != WorldPath::root() {
                snapshot.create_dir(directory.as_str())?;
            }
            for file in source.files(&directory)? {
                let contents = source.read(&file)?.ok_or_else(|| {
                    Error::World(format!(
                        "World file disappeared while taking snapshot: {}",
                        file.as_str()
                    ))
                })?;
                snapshot.insert_file(file.as_str(), contents)?;
            }
            let mut children = source.directories(&directory)?;
            children.reverse();
            pending.extend(children);
        }
        Ok(snapshot)
    }

    pub fn create_dir(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = WorldPath::new(path)?;
        if self.files.contains_key(&path) {
            return Err(Error::World(format!(
                "World path is both a file and directory: {}",
                path.as_str()
            )));
        }
        self.insert_parent_directories(&path)?;
        self.directories.insert(path);
        Ok(())
    }

    pub fn insert_file(&mut self, path: impl AsRef<Path>, source: impl Into<String>) -> Result<()> {
        let path = WorldPath::new(path)?;
        if path == WorldPath::root() {
            return Err(Error::World("World root cannot be a file".into()));
        }
        if self.directories.contains(&path) {
            return Err(Error::World(format!(
                "World path is both a file and directory: {}",
                path.as_str()
            )));
        }
        self.insert_parent_directories(&path)?;
        self.files.insert(path, source.into());
        Ok(())
    }

    fn insert_parent_directories(&mut self, path: &WorldPath) -> Result<()> {
        let mut parent = path.parent();
        while let Some(path) = parent {
            parent = path.parent();
            if self.files.contains_key(&path) {
                return Err(Error::World(format!(
                    "World file cannot contain another path: {}",
                    path.as_str()
                )));
            }
            self.directories.insert(path);
        }
        Ok(())
    }
}

impl WorldView for MemoryWorld {
    fn read(&self, path: &WorldPath) -> Result<Option<String>> {
        Ok(self.files.get(path).cloned())
    }

    fn is_file(&self, path: &WorldPath) -> Result<bool> {
        Ok(self.files.contains_key(path))
    }

    fn is_dir(&self, path: &WorldPath) -> Result<bool> {
        Ok(self.directories.contains(path))
    }

    fn directories(&self, path: &WorldPath) -> Result<Vec<WorldPath>> {
        Ok(self
            .directories
            .iter()
            .filter(|candidate| candidate.parent().as_ref() == Some(path))
            .cloned()
            .collect())
    }

    fn files(&self, path: &WorldPath) -> Result<Vec<WorldPath>> {
        Ok(self
            .files
            .keys()
            .filter(|candidate| candidate.parent().as_ref() == Some(path))
            .cloned()
            .collect())
    }
}

/// The effective World seen by SayoVM. A user-owned top-level Component
/// completely shadows the same standard Component; files are never merged
/// across those Component boundaries.
pub struct LayeredWorld<U, S> {
    user: U,
    standard: S,
}

impl<U, S> LayeredWorld<U, S> {
    pub fn new(user: U, standard: S) -> Self {
        Self { user, standard }
    }
}

impl<U: WorldView, S: WorldView> LayeredWorld<U, S> {
    fn layer_for(&self, path: &WorldPath) -> Result<&dyn WorldView> {
        match path.top_component() {
            Some(component)
                if self
                    .user
                    .is_dir(&WorldPath::new(component).expect("one valid component"))? =>
            {
                Ok(&self.user)
            }
            _ => Ok(&self.standard),
        }
    }
}

impl<U: WorldView, S: WorldView> WorldView for LayeredWorld<U, S> {
    fn read(&self, path: &WorldPath) -> Result<Option<String>> {
        self.layer_for(path)?.read(path)
    }

    fn is_file(&self, path: &WorldPath) -> Result<bool> {
        self.layer_for(path)?.is_file(path)
    }

    fn is_dir(&self, path: &WorldPath) -> Result<bool> {
        if path == &WorldPath::root() {
            return Ok(true);
        }
        self.layer_for(path)?.is_dir(path)
    }

    fn directories(&self, path: &WorldPath) -> Result<Vec<WorldPath>> {
        if path != &WorldPath::root() {
            return self.layer_for(path)?.directories(path);
        }
        let mut names = BTreeMap::new();
        for child in self.standard.directories(path)? {
            if let Some(name) = child.file_name() {
                names.insert(name.to_owned(), child);
            }
        }
        for child in self.user.directories(path)? {
            if let Some(name) = child.file_name() {
                names.insert(name.to_owned(), child);
            }
        }
        Ok(names.into_values().collect())
    }

    fn files(&self, path: &WorldPath) -> Result<Vec<WorldPath>> {
        self.layer_for(path)?.files(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_paths_that_escape_the_world() {
        assert!(WorldPath::new("../outside.sayo").is_err());
        assert!(WorldPath::new("/outside.sayo").is_err());
    }

    #[test]
    fn memory_world_preserves_empty_relation_directories() {
        let mut world = MemoryWorld::new();
        world.create_dir("Add/Int/Plus").unwrap();
        assert!(
            world
                .is_dir(&WorldPath::new("Add/Int/Plus").unwrap())
                .unwrap()
        );
        assert_eq!(
            world
                .directories(&WorldPath::new("Add/Int").unwrap())
                .unwrap(),
            vec![WorldPath::new("Add/Int/Plus").unwrap()]
        );
    }

    #[test]
    fn user_component_completely_shadows_standard_component() {
        let user = MemoryWorld::from_files([("Add/space.sayo", "user")]).unwrap();
        let standard = MemoryWorld::from_files([
            ("Add/space.sayo", "standard"),
            ("Add/Int/Plus/main.sayo", "%0"),
            ("Fold/space.sayo", "fold"),
        ])
        .unwrap();
        let world = LayeredWorld::new(user, standard);

        assert_eq!(
            world
                .read(&WorldPath::new("Add/space.sayo").unwrap())
                .unwrap(),
            Some("user".into())
        );
        assert!(
            !world
                .is_file(&WorldPath::new("Add/Int/Plus/main.sayo").unwrap())
                .unwrap()
        );
        assert_eq!(
            world
                .read(&WorldPath::new("Fold/space.sayo").unwrap())
                .unwrap(),
            Some("fold".into())
        );
    }

    #[test]
    fn snapshot_preserves_files_and_explicit_empty_directories() {
        let mut original = MemoryWorld::from_files([
            ("Add/space.sayo", "checker"),
            ("Add/Int/Computed/main.sayo", "%0"),
        ])
        .unwrap();
        original.create_dir("Add/Int/Captured").unwrap();

        let snapshot = MemoryWorld::snapshot(&original).unwrap();
        assert_eq!(
            snapshot
                .read(&WorldPath::new("Add/space.sayo").unwrap())
                .unwrap()
                .as_deref(),
            Some("checker")
        );
        assert!(
            snapshot
                .is_dir(&WorldPath::new("Add/Int/Captured").unwrap())
                .unwrap()
        );
    }

    #[test]
    fn virtual_paths_are_bound_to_their_world_snapshot() {
        let first = WorldHandle::new(MemoryWorld::from_files([("main.sayo", "first")]).unwrap());
        let second = WorldHandle::new(MemoryWorld::from_files([("main.sayo", "second")]).unwrap());
        let first_path = first.root().join("main.sayo").unwrap();
        let same_path = first.root().join("main.sayo").unwrap();
        let other_path = second.root().join("main.sayo").unwrap();

        assert_eq!(first_path, same_path);
        assert_ne!(first_path, other_path);
        assert_eq!(first_path.read().unwrap().as_deref(), Some("first"));
        assert_eq!(other_path.read().unwrap().as_deref(), Some("second"));
    }
}
