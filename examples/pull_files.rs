extern crate env_logger;
extern crate futures;
extern crate hubcaps;
extern crate tokio;

use std::env;

use tokio::runtime::Runtime;

use hubcaps::{Credentials, Github, Result};

fn main() -> Result<()> {
    drop(env_logger::init());
    match env::var("GITHUB_TOKEN").ok() {
        Some(token) => {
            let mut rt = Runtime::new()?;
            let github = Github::new(
                concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")),
                Credentials::Token(token),
            );
            for diff in rt.block_on(github.repo("rust-lang", "rust").pulls().get(49536).files())? {
                println!("{:#?}", diff);
            }
            Ok(())
        }
        _ => Err("example missing GITHUB_TOKEN".into()),
    }
}
