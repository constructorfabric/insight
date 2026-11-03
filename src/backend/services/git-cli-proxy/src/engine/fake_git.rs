use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, ExitStatus};
use std::time::Duration;

const PROBE_ARG: &str = "--fake-git-probe";
const SETTLE_ATTEMPTS: u32 = 200;
const SETTLE_PAUSE: Duration = Duration::from_millis(10);

pub(crate) fn write_fake_git(path: &Path, body: &str) {
    let source = format!("#!/bin/sh\nif [ \"$1\" = {PROBE_ARG} ]; then exit 0; fi\n{body}\n");
    let written = std::fs::write(path, source)
        .and_then(|()| std::fs::set_permissions(path, PermissionsExt::from_mode(0o755)));
    assert!(written.is_ok(), "write fake git {path:?}: {written:?}");

    // WORKAROUND: Linux exec fails with ETXTBSY while a child forked by a
    // parallel test still holds the write fd; one successful exec proves none is left.
    let settled = (0..SETTLE_ATTEMPTS).find_map(|_| {
        let probed = Command::new(path).arg(PROBE_ARG).status();
        let busy = matches!(&probed, Err(e) if e.kind() == ErrorKind::ExecutableFileBusy);
        if busy {
            std::thread::sleep(SETTLE_PAUSE);
        }
        (!busy).then_some(probed)
    });
    let ran = settled
        .as_ref()
        .is_some_and(|probed| probed.as_ref().is_ok_and(ExitStatus::success));
    assert!(ran, "fake git probe {path:?}: {settled:?}");
}
