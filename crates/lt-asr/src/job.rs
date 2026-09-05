//! Job Object 兜底（经验 E-05）：父进程任何方式退出（含崩溃）时 OS 收割 worker。

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

/// 持有 Job 句柄；Drop 关闭句柄 → KILL_ON_JOB_CLOSE 生效
pub struct JobHandle {
    handle: HANDLE,
}

impl JobHandle {
    /// 创建 kill-on-close 的 Job
    pub fn new() -> windows::core::Result<Self> {
        unsafe {
            let handle = CreateJobObjectW(None, None)?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )?;
            Ok(Self { handle })
        }
    }

    /// 把子进程绑定到 Job（必须在子进程启动后尽快，先于任何长任务）
    pub fn assign(&self, child: &std::process::Child) -> windows::core::Result<()> {
        use std::os::windows::io::AsRawHandle;
        let raw = child.as_raw_handle();
        unsafe { AssignProcessToJobObject(self.handle, HANDLE(raw)) }
    }
}

impl Drop for JobHandle {
    fn drop(&mut self) {
        // 关闭句柄触发 KILL_ON_JOB_CLOSE；失败无能为力（进程多半已退出）
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}
