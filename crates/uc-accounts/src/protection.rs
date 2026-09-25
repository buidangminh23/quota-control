#[cfg(not(unix))]
use crate::AccountError;
use crate::Result;

#[cfg(windows)]
fn transform(bytes: &[u8], entropy: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: entropy.len() as u32,
        pbData: entropy.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let success = unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if success == 0 {
        return Err(if encrypt {
            AccountError::Protection
        } else {
            AccountError::CredentialsUnavailable
        });
    }
    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        std::ptr::write_bytes(output.pbData, 0, output.cbData as usize);
        LocalFree(output.pbData.cast());
    }
    Ok(result)
}

pub fn protect(bytes: &[u8], entropy: &[u8]) -> Result<Vec<u8>> {
    #[cfg(windows)]
    {
        transform(bytes, entropy, true)
    }
    #[cfg(unix)]
    {
        let _ = entropy;
        Ok(bytes.to_vec())
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (bytes, entropy);
        Err(AccountError::Protection)
    }
}

pub fn unprotect(bytes: &[u8], entropy: &[u8]) -> Result<Vec<u8>> {
    #[cfg(windows)]
    {
        transform(bytes, entropy, false)
    }
    #[cfg(unix)]
    {
        let _ = entropy;
        Ok(bytes.to_vec())
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (bytes, entropy);
        Err(AccountError::CredentialsUnavailable)
    }
}
