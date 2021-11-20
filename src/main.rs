use std::path::PathBuf;

use git2::{Commit, Repository};
use structopt::StructOpt;

#[derive(StructOpt)]
struct Args {
    #[structopt(name = "original")]
    arg_original: String,
    #[structopt(name = "target")]
    arg_target: Option<String>,
    #[structopt(name = "username")]
    arg_username: Option<String>,
    #[structopt(name = "email")]
    arg_email: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::from_args();
    println!("Try to open a repository at {}", &args.arg_original);

    let path = PathBuf::from(&args.arg_original);
    let repo = Repository::open(&path)?;

    let mut commit: Option<Commit> = None;
    for reference in repo.references()? {
        match reference {
            // TODO(feature): Handle references other than refs/heads/main
            Ok(ref reference) if reference.name().unwrap().contains("heads/main") => {
                commit = Some(reference.peel_to_commit()?);
            }
            _ => continue
        }
    };
    let mut commit = commit.unwrap();

    loop {
        println!("{:?}", commit);
        match commit.parent(0 /* FIXME? */) {
            Ok(parent) => {
                commit = parent;
            },
            Err(_) => break,
        }
    }

    Ok(())
}
