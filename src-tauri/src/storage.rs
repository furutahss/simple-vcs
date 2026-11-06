use std::{
    io::{Read, Write},
    path::Path,
};
use vcs_core::{Error, Result};

pub fn read(path: &Path) -> Result<Option<Vec<u8>>> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(Error::new(
                "storage",
                "記録ファイルを読み取れません。権限を確認してください",
            ))
        }
    };
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::new("storage", "記録の読取りに失敗しました"))?;
    if bytes.len() > 1024 * 1024 {
        return Err(Error::new("storage", "記録ファイルが上限を超えています"));
    }
    Ok(Some(bytes))
}

pub fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    if bytes.len() > 1024 * 1024 {
        return Err(Error::new("storage", "記録ファイルが上限を超えています"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| Error::new("storage", "記録の保存先が不正です"))?;
    let failed = |_| {
        Error::new(
            "storage",
            "記録を保存できません。空き容量と権限を確認してください",
        )
    };
    std::fs::create_dir_all(parent).map_err(failed)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(failed)?;
    temp.write_all(bytes).map_err(failed)?;
    temp.as_file().sync_all().map_err(failed)?;
    temp.persist(path)
        .map_err(|_| Error::new("storage", "記録を置換できません。以前の記録を保持します"))?;
    #[cfg(unix)]
    std::fs::File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(failed)?;
    Ok(())
}
