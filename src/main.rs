use anyhow::Result;
use capnpc_elm::generate_elm_code;
use std::io::{self, BufReader};

fn main() -> Result<()> {
    // 从标准输入读取
    let stdin = io::stdin();
    let reader = BufReader::new(stdin.lock());

    // 生成Elm代码
    generate_elm_code(reader)?;

    Ok(())
}
