//! 子进程运行辅助：stdout/stderr 逐行捕获并经回调转发 + 可取消。
//!
//! core 层不依赖任何 UI/事件框架——行的转发目标由调用方以闭包注入
//! （GUI 转成事件、CLI 直接 println）。此前的实现继承 stdio，GUI 里迁移
//! 零反馈且无法中断。

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

/// 取消时统一使用的错误信息（拼进各迁移报告的 error 字段）。
pub(crate) const CANCEL_MSG: &str = "已被用户取消";

/// 单次子进程运行的结局。
#[derive(Debug)]
pub(crate) enum StreamOutcome {
    /// 子进程自然退出（成功与否由调用方看 ExitStatus）
    Exited(std::process::ExitStatus),
    /// cancel 置位后子进程被 kill
    Cancelled,
}

/// 运行命令：stdout/stderr 均改为 piped 逐行读，每行经 `on_line` 转发；
/// `cancel` 置位即 kill 子进程并停止转发。
///
/// - `Err` 仅表示 spawn 失败（如命令不存在）；
/// - 两个流的读线程只负责送行进 channel，转发统一在调用线程完成，
///   因此 `on_line` 无需 `Send`/`Sync`，也不会与自身并发。
/// - 取消检查间隔 200ms：子进程静默期（如下载中）也能及时中断。
pub(crate) fn run_streamed(
    program: &str,
    args: &[String],
    dir: &Path,
    cancel: &AtomicBool,
    mut on_line: impl FnMut(&str),
) -> Result<StreamOutcome, String> {
    let mut child = Command::new(program)
        .args(args)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法执行 `{program}`: {e}"))?;

    let (tx, rx) = mpsc::channel::<String>();
    let mut readers = Vec::new();
    // stdout 与 stderr 类型不同（ChildStdout/ChildStderr），各自挂一个读线程
    fn spawn_reader<S: std::io::Read + Send + 'static>(
        stream: S,
        tx: mpsc::Sender<String>,
        readers: &mut Vec<std::thread::JoinHandle<()>>,
    ) {
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(|l| l.ok()) {
                if tx.send(line).is_err() {
                    break; // 接收端已放弃
                }
            }
        }));
    }
    if let Some(out) = child.stdout.take() {
        spawn_reader(out, tx.clone(), &mut readers);
    }
    if let Some(err) = child.stderr.take() {
        spawn_reader(err, tx.clone(), &mut readers);
    }
    drop(tx); // 读线程全部结束后 channel 断开，recv 返回 Disconnected

    let mut cancelled = false;
    loop {
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            let _ = child.kill();
            break;
        }
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => on_line(&line),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait();
    for r in readers {
        let _ = r.join();
    }
    if cancelled {
        return Ok(StreamOutcome::Cancelled);
    }
    status
        .map(StreamOutcome::Exited)
        .map_err(|e| format!("等待 `{program}` 退出失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 跨平台的"打印两行"命令。
    fn echo_cmd() -> (&'static str, Vec<String>) {
        if cfg!(windows) {
            ("cmd", vec!["/C".into(), "echo hello".into(), "& echo world".into()])
        } else {
            ("sh", vec!["-c".into(), "echo hello; echo world".into()])
        }
    }

    /// 跨平台的"跑 30 秒"命令。
    fn sleep_cmd() -> (&'static str, Vec<String>) {
        if cfg!(windows) {
            ("ping", vec!["-n".into(), "30".into(), "127.0.0.1".into()])
        } else {
            ("sleep", vec!["30".into()])
        }
    }

    #[test]
    fn streams_and_forwards_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let (prog, args) = echo_cmd();
        let cancel = AtomicBool::new(false);
        let seen = std::sync::Mutex::new(Vec::new());
        let out = run_streamed(prog, &args, tmp.path(), &cancel, |l| {
            seen.lock().unwrap().push(l.to_string());
        })
        .unwrap();
        match out {
            StreamOutcome::Exited(s) => assert!(s.success(), "echo 应成功: {s}"),
            StreamOutcome::Cancelled => panic!("未取消却被判为 Cancelled"),
        }
        let lines = seen.into_inner().unwrap();
        assert!(
            lines.iter().any(|l| l.contains("hello")),
            "stdout 行应被转发: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("world")),
            "第二行也应被转发: {lines:?}"
        );
    }

    #[test]
    fn pre_set_cancel_kills_child() {
        let tmp = tempfile::tempdir().unwrap();
        let (prog, args) = sleep_cmd();
        let cancel = AtomicBool::new(true); // 一开始就取消
        let start = std::time::Instant::now();
        let out = run_streamed(prog, &args, tmp.path(), &cancel, |_| {}).unwrap();
        assert!(matches!(out, StreamOutcome::Cancelled), "应判为 Cancelled");
        // 应在远小于 30s 内返回（200ms 轮询 + kill + wait）
        assert!(
            start.elapsed().as_secs() < 10,
            "取消应迅速终止子进程，实际耗时 {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn missing_program_is_spawn_error() {
        let tmp = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        let res = run_streamed(
            "definitely-not-a-real-program-xyz",
            &["--version".to_string()],
            tmp.path(),
            &cancel,
            |_| {},
        );
        assert!(res.is_err(), "spawn 失败应返回 Err: {res:?}");
    }
}
