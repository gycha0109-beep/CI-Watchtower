//! Cooperative cross-process ownership for a WatchTower SQLite writer.
//! This is a *guard*, not a migration: every writer must adopt the same lease.
//! Existing unmodified 0.3.40 desktop releases do NOT know about this lease.
use anyhow::{anyhow,Result};
use std::{fs::{File,OpenOptions,TryLockError},path::{Path,PathBuf}};

pub struct WriterLease{
    // Holding the OS file lock for the full writer lifetime is essential.
    // Never delete a lock file on drop: unlink/recreate races break mutual exclusion.
    _file:File,
    path:PathBuf
}

impl WriterLease{
    pub fn try_acquire(database:&Path)->Result<Self>{
        if !database.is_absolute() {return Err(anyhow!("WRITER_PATH_INVALID"));}
        let parent=database.parent().ok_or_else(||anyhow!("WRITER_PATH_INVALID"))?
            .canonicalize().map_err(|_|anyhow!("WRITER_PATH_INVALID"))?;
        let name=database.file_name().ok_or_else(||anyhow!("WRITER_PATH_INVALID"))?
            .to_str().ok_or_else(||anyhow!("WRITER_PATH_INVALID"))?;
        if name.is_empty() || name=="." || name==".."{
            return Err(anyhow!("WRITER_PATH_INVALID"));
        }
        let lock=parent.join(format!("{name}.ci-core-writer.lock"));
        let file=OpenOptions::new().read(true).write(true).create(true).open(&lock)
            .map_err(|_|anyhow!("WRITER_LEASE_UNAVAILABLE"))?;
        match file.try_lock(){
            Ok(())=>Ok(Self{_file:file,path:lock}),
            Err(TryLockError::WouldBlock)=>Err(anyhow!("WRITER_LEASE_HELD")),
            Err(TryLockError::Error(_))=>Err(anyhow!("WRITER_LEASE_UNAVAILABLE"))
        }
    }
    pub fn lock_path(&self)->&Path{&self.path}
}
#[cfg(test)]
mod tests{
    use super::*;
    fn temp_db(suffix:&str)->PathBuf{
        let unique=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .unwrap().as_nanos();
        std::env::temp_dir().join(format!("ci-core-lease-{}-{unique}-{suffix}.sqlite3",std::process::id()))
    }
    #[test]
    fn only_one_owner_per_database_then_release_on_drop(){
        let path=temp_db("same");
        let first=WriterLease::try_acquire(&path).unwrap();
        assert_eq!(WriterLease::try_acquire(&path).err().unwrap().to_string(),"WRITER_LEASE_HELD");
        let lock=first.lock_path().to_owned();
        drop(first);
        let second=WriterLease::try_acquire(&path).unwrap();
        drop(second);
        std::fs::remove_file(lock).unwrap();
        assert!(!path.exists());
    }
    #[test]
    fn independent_databases_do_not_conflict(){
        let a=temp_db("a");
        let b=temp_db("b");
        let one=WriterLease::try_acquire(&a).unwrap();
        let two=WriterLease::try_acquire(&b).unwrap();
        let files=[one.lock_path().to_owned(),two.lock_path().to_owned()];
        drop(two);drop(one);
        for file in files {std::fs::remove_file(file).unwrap();}
        assert!(!a.exists()&&!b.exists());
    }
    #[test]
    fn rejects_relative_path_without_creating_database(){
        assert_eq!(WriterLease::try_acquire(Path::new("relative.sqlite3"))
            .err().unwrap().to_string(),"WRITER_PATH_INVALID");
    }
    #[test]
    fn errors_do_not_contain_sensitive_file_paths(){
        let x=WriterLease::try_acquire(Path::new("/definitely-unavailable/opaque-token.sqlite3"));
        assert!(x.is_err());
        assert!(!x.err().unwrap().to_string().contains("opaque-token"));
    }
}
