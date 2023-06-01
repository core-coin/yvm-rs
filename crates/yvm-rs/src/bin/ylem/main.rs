use std::{env, process::Command};

fn main() -> anyhow::Result<()> {
    let args = env::args().skip(1).collect::<Vec<String>>();

    let version = yvm_lib::current_version()?.ok_or(yvm_lib::YlemVmError::GlobalVersionNotSet)?;
    let mut version_path = yvm_lib::version_path(version.to_string().as_str());
    version_path.push(format!("ylem-{}", version.to_string().as_str()));

    let status = Command::new(version_path).args(args).status()?;
    let code = status.code().unwrap_or(-1);
    std::process::exit(code);
}
