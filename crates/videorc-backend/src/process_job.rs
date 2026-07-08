use std::io;
use std::process::{Child as StdChild, Command as StdCommand, ExitStatus, Output as ProcessOutput};

use tokio::process::{Child as TokioChild, Command as TokioCommand};

pub fn spawn_owned_tokio(command: &mut TokioCommand) -> io::Result<TokioChild> {
    let mut child = command.spawn()?;
    if let Err(error) = assign_tokio_child(&child) {
        let _ = child.start_kill();
        return Err(error);
    }
    Ok(child)
}

pub fn spawn_owned_std(command: &mut StdCommand) -> io::Result<StdChild> {
    let mut child = command.spawn()?;
    if let Err(error) = assign_std_child(&child) {
        let _ = child.kill();
        return Err(error);
    }
    Ok(child)
}

pub async fn output_owned_tokio(command: &mut TokioCommand) -> io::Result<ProcessOutput> {
    spawn_owned_tokio(command)?.wait_with_output().await
}

pub async fn status_owned_tokio(command: &mut TokioCommand) -> io::Result<ExitStatus> {
    let mut child = spawn_owned_tokio(command)?;
    child.wait().await
}

pub fn output_owned_std(command: &mut StdCommand) -> io::Result<ProcessOutput> {
    spawn_owned_std(command)?.wait_with_output()
}

#[cfg(not(target_os = "windows"))]
fn assign_tokio_child(_child: &TokioChild) -> io::Result<()> {
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn assign_std_child(_child: &StdChild) -> io::Result<()> {
    Ok(())
}

#[cfg(target_os = "windows")]
mod windows_job {
    use std::io;
    use std::os::windows::io::{AsRawHandle, RawHandle};
    use std::sync::OnceLock;

    use tokio::process::Child as TokioChild;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    use windows::core::PCWSTR;

    use super::StdChild;

    static BACKEND_JOB_HANDLE: OnceLock<Result<usize, String>> = OnceLock::new();

    pub(super) fn assign_tokio_child(child: &TokioChild) -> io::Result<()> {
        let raw_handle = child
            .raw_handle()
            .ok_or_else(|| io::Error::other("spawned child exited before Job Object assignment"))?;
        assign_raw_process_handle(raw_handle)
    }

    pub(super) fn assign_std_child(child: &StdChild) -> io::Result<()> {
        assign_raw_process_handle(child.as_raw_handle())
    }

    fn assign_raw_process_handle(raw_handle: RawHandle) -> io::Result<()> {
        let job = backend_job_handle()?;
        let process = HANDLE(raw_handle);
        unsafe { AssignProcessToJobObject(job, process) }.map_err(|error| {
            io::Error::other(format!("Could not assign child to Job Object: {error}"))
        })
    }

    fn backend_job_handle() -> io::Result<HANDLE> {
        match BACKEND_JOB_HANDLE.get_or_init(create_backend_job_handle) {
            Ok(raw) => Ok(HANDLE(*raw as *mut core::ffi::c_void)),
            Err(message) => Err(io::Error::other(message.clone())),
        }
    }

    fn create_backend_job_handle() -> Result<usize, String> {
        let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }
            .map_err(|error| format!("Could not create backend Job Object: {error}"))?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
        .map_err(|error| format!("Could not configure backend Job Object: {error}"))?;
        Ok(job.0 as usize)
    }
}

#[cfg(target_os = "windows")]
fn assign_tokio_child(child: &TokioChild) -> io::Result<()> {
    windows_job::assign_tokio_child(child)
}

#[cfg(target_os = "windows")]
fn assign_std_child(child: &StdChild) -> io::Result<()> {
    windows_job::assign_std_child(child)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quick_exit_command() -> StdCommand {
        #[cfg(target_os = "windows")]
        {
            let mut command = StdCommand::new("cmd");
            command.args(["/C", "exit", "0"]);
            command
        }

        #[cfg(not(target_os = "windows"))]
        {
            StdCommand::new("true")
        }
    }

    #[test]
    fn owned_std_child_can_exit_cleanly() {
        let mut command = quick_exit_command();
        let status = spawn_owned_std(&mut command)
            .expect("child should spawn")
            .wait()
            .expect("child should wait");
        assert!(status.success());
    }
}
