use command_group::GroupChild;

#[cfg(target_os = "windows")]
pub fn assign_to_job_object(child: &mut GroupChild) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use std::ptr;

    use winapi::um::handleapi::CloseHandle;
    use winapi::um::jobapi2::*;
    use winapi::um::winnt::*;

    unsafe {
        let job = CreateJobObjectW(ptr::null_mut(), ptr::null());
        if job.is_null() {
            return Err(std::io::Error::last_os_error());
        }

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        let result = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &mut info as *mut _ as *mut _,
            std::mem::size_of_val(&info) as u32,
        );

        if result == 0 {
            CloseHandle(job);
            return Err(std::io::Error::last_os_error());
        }

        let handle = child.inner().as_raw_handle() as *mut winapi::ctypes::c_void;
        if AssignProcessToJobObject(job, handle) == 0 {
            CloseHandle(job);
            return Err(std::io::Error::last_os_error());
        }

        let _ = job;
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
pub fn assign_to_job_object(_child: &mut GroupChild) -> std::io::Result<()> {
    Ok(())
}
