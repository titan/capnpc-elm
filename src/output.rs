use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

/// 渲染输出策略 trait：抽象文件写入行为
pub trait OutputWriter {
    fn write(&self, path: &Path, content: &str) -> anyhow::Result<()>;
}

/// 文件系统写入器：创建目录 + elm-format + 写文件
pub struct FileWriter;

impl OutputWriter for FileWriter {
    fn write(&self, path: &Path, content: &str) -> anyhow::Result<()> {
        // 确保目录存在
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = File::create(path)?;
        write!(file, "{}", format_elm_code(content))?;
        Ok(())
    }
}

/// 内存写入器：收集内容到 HashMap（无格式化，便于测试）
pub struct MemoryWriter {
    contents: RefCell<HashMap<PathBuf, String>>,
}

impl MemoryWriter {
    pub fn new() -> Self {
        Self {
            contents: RefCell::new(HashMap::new()),
        }
    }

    /// 获取所有已写入的内容
    pub fn get_all(&self) -> HashMap<PathBuf, String> {
        self.contents.borrow().clone()
    }
}

impl OutputWriter for MemoryWriter {
    fn write(&self, path: &Path, content: &str) -> anyhow::Result<()> {
        self.contents
            .borrow_mut()
            .insert(path.to_path_buf(), content.to_string());
        Ok(())
    }
}

/// 尝试使用 elm-format 格式化 Elm 代码。
///
/// 如果 elm-format 未找到或失败，返回原始代码并打印警告到 stderr。
pub fn format_elm_code(unformatted_code: &str) -> String {
    match std::process::Command::new("elm-format")
        .arg("--yes")
        .arg("--stdin")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(mut child) => {
            if let Some(mut stdin) = child.stdin.take() {
                if stdin.write_all(unformatted_code.as_bytes()).is_err() {
                    return unformatted_code.to_string();
                }
            } else {
                return unformatted_code.to_string();
            }

            match child.wait_with_output() {
                Ok(output) => {
                    if output.status.success() {
                        match String::from_utf8(output.stdout) {
                            Ok(formatted_code) => formatted_code,
                            Err(_) => unformatted_code.to_string(),
                        }
                    } else {
                        let _ = String::from_utf8_lossy(&output.stderr);
                        unformatted_code.to_string()
                    }
                }
                Err(_) => unformatted_code.to_string(),
            }
        }
        Err(_) => unformatted_code.to_string(),
    }
}
