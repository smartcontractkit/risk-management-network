use std::process::Command;

fn main() {
    let git_hash = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|x| String::from_utf8(x.stdout).ok());

    println!("cargo:rustc-env=GIT_HASH={:?}", git_hash);

    prost_build::compile_protos(
        &[
            "proto/rageproxy.proto",
            "proto/rmn_offchain.proto",
            "proto/keystore.proto",
            "proto/offchainreporting3_offchain_config.proto",
        ],
        &["proto/"],
    )
    .unwrap();
}
