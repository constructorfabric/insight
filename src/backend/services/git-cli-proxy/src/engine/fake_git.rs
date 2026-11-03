use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

const PROBE_ARG: &str = "--fake-git-probe";
const SETTLE_ATTEMPTS: u32 = 200;
const SETTLE_PAUSE: Duration = Duration::from_millis(10);

pub(crate) fn write_fake_git(path: &Path, body: &str) {
    let source = format!("#!/bin/sh\nif [ \"$1\" = {PROBE_ARG} ]; then exit 0; fi\n{body}\n");
    if let Err(e) = std::fs::write(path, source) {
        panic!("write fake git {}: {e}", path.display());
    }
    if let Err(e) = std::fs::set_permissions(path, PermissionsExt::from_mode(0o755)) {
        panic!("chmod fake git {}: {e}", path.display());
    }

    // WORKAROUND: Linux exec fails with ETXTBSY while a child forked by a
    // parallel test still holds the write fd; one successful exec proves none is left.
    for _ in 0..SETTLE_ATTEMPTS {
        match Command::new(path).arg(PROBE_ARG).status() {
            Ok(status) if status.success() => return,
            Ok(status) => panic!("fake git probe {} exited with {status}", path.display()),
            Err(e) if e.kind() == ErrorKind::ExecutableFileBusy => std::thread::sleep(SETTLE_PAUSE),
            Err(e) => panic!("fake git probe {}: {e}", path.display()),
        }
    }
    panic!("fake git {} stayed busy for writing", path.display());
}
