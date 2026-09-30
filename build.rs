use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    println!("cargo:rerun-if-env-changed=ADD_BOT_COMMIT");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    for path in ["HEAD", "refs/heads", "packed-refs"] {
        if let Some(path) = git(&["rev-parse", "--git-path", path]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    let commit = std::env::var("ADD_BOT_COMMIT")
        .ok()
        .or_else(|| std::env::var("GITHUB_SHA").ok())
        .or_else(|| git(&["rev-parse", "HEAD"]));
    let commit = commit.filter(|value| {
        (7..=40).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_hexdigit())
    });
    println!(
        "cargo:rustc-env=ADD_BOT_BUILD_COMMIT={}",
        commit
            .map(|value| value[..7].to_owned())
            .unwrap_or_else(|| "unknown".into())
    );
}
