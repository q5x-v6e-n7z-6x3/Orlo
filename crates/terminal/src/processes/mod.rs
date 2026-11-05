//! Shell and descendant process cleanup.

#[cfg(target_os = "macos")]
fn terminate_shell_jobs(pid: libc::pid_t) {
    if pid <= 0 || unsafe { libc::kill(pid, libc::SIGSTOP) } != 0 {
        return;
    }
    // Freeze each parent before listing its children, so jobs cannot fork past cleanup.
    let mut processes = vec![pid];
    let mut at = 0;
    while at < processes.len() {
        let mut children = vec![0 as libc::pid_t; 32];
        loop {
            let count = unsafe {
                libc::proc_listchildpids(
                    processes[at],
                    children.as_mut_ptr().cast(),
                    std::mem::size_of_val(children.as_slice()) as libc::c_int,
                )
            }
            .max(0) as usize;
            if count < children.len() {
                children.truncate(count);
                break;
            }
            children.resize(children.len() * 2, 0);
        }
        for child in children.into_iter().filter(|child| *child > 0) {
            if !processes.contains(&child) && unsafe { libc::kill(child, libc::SIGSTOP) } == 0 {
                processes.push(child);
            }
        }
        at += 1;
    }
    for child in processes.into_iter().skip(1).rev() {
        unsafe {
            libc::kill(child, libc::SIGKILL);
        }
    }
    unsafe {
        libc::kill(pid, libc::SIGCONT);
    }
}

/// Stop the shell and its jobs before releasing its controlling terminal.
pub(crate) fn terminate_shell(master: &dyn portable_pty::MasterPty, child: &mut dyn portable_pty::Child, pid: u32) {
    #[cfg(target_os = "macos")]
    terminate_shell_jobs(pid as libc::pid_t);
    // Foreground jobs can have a different process group from their login shell.
    if let Some(group) = master.process_group_leader().filter(|group| *group > 0) {
        unsafe {
            libc::kill(-group, libc::SIGHUP);
        }
    }
    if pid == 0 {
        let _ = child.kill();
        return;
    }
    // portable-pty's Child::kill can wait 200ms after HUP, longer than GPUI's 100ms
    // application-quit grace. Give normal shell/history cleanup 50ms ourselves, then
    // force termination. A reaped child must never receive further PID/group signals.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGHUP);
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
    loop {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    {
        unsafe {
            libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }
}
