use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
};

use git2::{Commit, Index, ObjectType, Oid, Repository, Tree, TreeEntry, TreeWalkMode, TreeWalkResult};
use structopt::StructOpt;

#[derive(StructOpt)]
struct Args {
    #[structopt(name = "original")]
    arg_original: String,
    #[structopt(name = "target")]
    arg_target: String,
    #[structopt(name = "name")]
    arg_name: String,
    #[structopt(name = "email")]
    arg_email: String,
}

struct TargetRepository<'a> {
    source: &'a Repository,
    inner: Repository,
    root: String,
    _name: String,
    _email: String,
    path_to_oid: BTreeMap<PathBuf, Oid>,
}

impl<'a> TargetRepository<'a> {
    fn new(source: &'a Repository, path: &str, name: &str, email: &str) -> anyhow::Result<Self> {
        let repo = Repository::init(path)?;
        Ok(TargetRepository {
            source,
            inner: repo,
            root: String::from(path),
            _name: String::from(name),
            _email: String::from(email),
            path_to_oid: Default::default(),
        })
    }

    fn handle_tree(
        &mut self,
        tree: &Tree,
    ) -> anyhow::Result<()> {
        tree.walk(TreeWalkMode::PostOrder, |_, entry| {
            // how to get a full path name?
            // We need to know the directory structure, but `.name()`
            // only gives us the file name w/o its parent directories.
            println!("{}", entry.name().unwrap());
            TreeWalkResult::Ok
        })?;

        Ok(())
    }

    /// Recursively copy blob contents.
    fn handle_tree_entry(
        &mut self,
        tent: TreeEntry,
        pathbuf: &PathBuf,
        index: &Index,
    ) -> anyhow::Result<()> {
        let name = tent.name().unwrap();
        let obj = tent.to_object(&self.source)?;
        let mut pathbuf = pathbuf.clone();
        pathbuf.push(name);

        match obj.kind() {
            Some(ObjectType::Blob) => {
                // Avoid copying multiple times by memoizing
                // the previous Oid that is copied before.
                let entry = self.path_to_oid.get(&pathbuf);

                if entry.is_none() || *entry.unwrap() != obj.id() {
                    // XXX: How to force udpate with `write`?
                    let mut root = PathBuf::from(&self.root);
                    root.push(&pathbuf);
                    println!("{:?}", &root);
                    std::fs::write(
                        &root,
                        obj.as_blob().unwrap().content())?;

                    // TODO: Update index
                    let index = &mut self.inner.index()?;
                    index.add_path(&pathbuf)?;

                    self.path_to_oid.insert(pathbuf, obj.id());
                }
            },
            Some(ObjectType::Tree) => {
                let tree = obj.as_tree().unwrap();
                let mut root = PathBuf::from(&self.root);
                root.push(&pathbuf);
                if !root.exists() {
                    std::fs::create_dir(&root)?; // Ignore if already exists.
                }
                for child in tree.iter() {
                    self.handle_tree_entry(child, &pathbuf, &index)?;
                }
                // TODO: Update index?
                // TODO: What if we ignore the directories?
                // let index = &mut self.inner.index()?;
                // index.add_path(&pathbuf)?;
            },
            _ => unimplemented!(),
        }

        Ok(())
    }

    /// Handle a commit from the source repository.
    ///
    /// 1. Copies blobs (that are reachable from the tree) from the
    ///    source repository to the destination repository.
    /// 2. Update index in the destination repositories with updated
    ///    blobs.
    /// 3. Commit in the destination repository.
    ///
    /// Returns a Oid so that we can create a reference (e.g., main).
    fn handle_commit(
        &mut self,
        commit: &Commit,
        parents: Option<&[Oid]>,
    ) -> anyhow::Result<Oid> {
        let tree = commit.tree()?;
        let pathbuf = PathBuf::new();
        let index = self.inner.index()?;

        for tent in tree.iter() {
            self.handle_tree_entry(tent, &pathbuf, &index)?;
        }

        let sig = self.source.signature()?;
        let index = &mut self.inner.index()?;
        let tree_id = index.write_tree()?;
        let tree = self.inner.find_tree(tree_id)?;
        let parents = parents
            .and_then(|oids| {
                Some(oids.iter()
                    .map(|oid| {
                        self.inner.find_commit(*oid).unwrap()
                    })
                    .collect::<Vec<_>>())
            })
            .unwrap_or(vec![]);
        let mut hack = vec![]; // Maybe use a macro?
        for i in 0..parents.len() {
            hack.push(&parents[i]);
        }
        Ok(self.inner.commit(Some("HEAD"),
                             &sig, &sig,
                             commit.message().unwrap(),
                             &tree, &hack[..])?)
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::from_args();

    let path = PathBuf::from(&args.arg_original);
    let repo = Repository::open(&path)?;

    let mut commit: Option<Commit> = None;
    for reference in repo.references()? {
        match reference {
            // TODO(feature): Handle references other than refs/heads/main
            Ok(ref reference) if reference.name().unwrap().contains("heads/master") => {
                commit = Some(reference.peel_to_commit()?);
            }
            _ => continue
        }
    };
    // TODO(ugly): Handle the case where the name of primary branch is main (instead of master).
    if commit.is_none() {
        for reference in repo.references()? {
            match reference {
                // TODO(feature): Handle references other than refs/heads/main
                Ok(ref reference) if reference.name().unwrap().contains("heads/main") => {
                    commit = Some(reference.peel_to_commit()?);
                }
                _ => continue
            }
        };
    }
    let mut commit = commit.unwrap();

    // 1. Push commits from the original repository to a stack.
    let mut orig_commits = VecDeque::new();
    loop {
        let tmp = &commit;
        orig_commits.push_back(tmp.clone());
        // Current implementation can only handle *linear* history at best.
        match commit.parent(0 /* FIXME? */) {
            Ok(parent) => {
                commit = parent;
            },
            Err(_) => break,
        }
    }

    // 2. Initialize a target repository.
    let mut target = TargetRepository::new(
        &repo,
        &args.arg_target,
        &args.arg_name,
        &args.arg_email,
    )?;

    // XXX: This will copy *blobs* that are added by the initial
    // commit of the soruce repository to the target repository
    // directories (Note this respects the directory architecture
    // as well).
    if let Some(initial_commit) = orig_commits.pop_back() {
        let target_initial_commit_oid = target.handle_commit(&initial_commit, None)?;
        if let Some(second_oldest_commit) = orig_commits.pop_back() {
            let target_initial_commit = Oid::zero();
            target.handle_commit(
                &second_oldest_commit,
                Some(&[target_initial_commit_oid]),
            )?;
        }
    }


    Ok(())
}
