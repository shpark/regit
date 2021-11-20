use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
};

use git2::{Commit, ObjectType, Oid, Repository};
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
    _path_to_oid: BTreeMap<String, Oid>,
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
            _path_to_oid: Default::default(),
        })
    }

    fn handle_commit(&mut self, commit: Commit) -> anyhow::Result<()> {
        let tree = commit.tree()?;
        let _index = self.inner.index()?;

        // Let's ingnore directories for now.
        for tent in tree.iter() {
            let name = tent.name().unwrap();
            let obj = tent.to_object(&self.source)?;
            match &obj.kind() {
                Some(ObjectType::Blob) => {
                    let mut path = PathBuf::from(&self.root);
                    path.push(name);
                    std::fs::write(
                        path,
                        obj.as_blob().unwrap().content())?;
                },
                _ => continue
            }
        }

        Ok(())
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

    // XXX: This will copy *blobs* (directories are now ignored) that
    // are added by the initial commit of the soruce repository to
    // the target repository directories.
    if let Some(initial_commit) = orig_commits.pop_back() {
        target.handle_commit(initial_commit)?;
    }

    Ok(())
}
