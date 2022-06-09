use std::path::Path;

use sha1::Sha1;
use sha2::Sha256;
use sha2::Digest;
pub enum MyHash {
    MD5(String),
    SHA256(String),
    SHA1(String),
    None
}

impl MyHash {
    pub fn validate(&self, file: &Path) -> crate::Result<bool> {
        let file_bytes = std::fs::read(file)?;
        match self {
            Self::MD5(h) => Ok(h == &format!("{:32x}", md5::compute(file_bytes))),
            Self::SHA256(h) => {
                let mut hasher = Sha256::new();
                hasher.update(file_bytes);
                let result = hasher.finalize();
                
                Ok(h == &format!("{:64x}", result))
                // todo!();
            }
            Self::SHA1(h) => {
                let mut hasher = Sha1::new();
                hasher.update(file_bytes);
                let result = hasher.finalize();
                
                Ok(h == &format!("{:32x}", result))
            }
            Self::None => Ok(false)
        }
    }
}
