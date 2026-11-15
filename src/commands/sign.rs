//! Key generation, signing and verification commands.

use std::io::Error;
use std::path::PathBuf;

use crate::cli_io::{AtomicTarget, open_input};
use dayzmake_core::error;
use dayzmake_core::error::ErrorExt;
use dayzmake_core::pbo::PBO;
use dayzmake_core::sign::{BIPrivateKey, BIPublicKey, BISign, BISignVersion};

/// Generates a key pair with the given name.
///
/// The output paths are created by appending extensions to the keyname.
///
/// # Errors
///
/// Returns an error if the key cannot be generated or a target file exists without `force`.
#[expect(
    clippy::needless_pass_by_value,
    reason = "public API: callers pass owned paths"
)]
pub fn cmd_keygen(keyname: PathBuf, force: bool) -> Result<(), Error> {
    let file_name = keyname
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| {
            error!(
                "Invalid key name \"{}\" (must be a UTF-8 file name).",
                keyname.display()
            )
        })?;
    let private_key = BIPrivateKey::generate(1024, file_name.to_string())?;
    let public_key = private_key.to_public_key()?;
    let name = file_name;

    let mut private_key_path = keyname.clone();
    private_key_path.set_file_name(format!("{name}.biprivatekey"));
    if !force && private_key_path.exists() {
        return Err(error!(
            "Target file \"{}\" already exists. Use --force to overwrite.",
            private_key_path.display()
        ));
    }
    let mut private_file = AtomicTarget::create(&private_key_path, true)
        .prepend_error("Failed to create private key:")?;
    private_key
        .write(&mut private_file)
        .prepend_error("Failed to write private key:")?;

    let mut public_key_path = keyname.clone();
    public_key_path.set_file_name(format!("{name}.bikey"));
    if !force && public_key_path.exists() {
        return Err(error!(
            "Target file \"{}\" already exists. Use --force to overwrite.",
            public_key_path.display()
        ));
    }
    let mut public_file = AtomicTarget::create(&public_key_path, true)
        .prepend_error("Failed to create public key:")?;
    public_key
        .write(&mut public_file)
        .prepend_error("Failed to write public key:")?;
    private_file.commit()?;
    public_file.commit()?;

    Ok(())
}

/// Signs a PBO with the given private key.
///
/// If the signature path is not given it is inferred from the PBO path.
///
/// # Errors
///
/// Returns an error if the key, PBO or signature file cannot be handled.
#[expect(
    clippy::needless_pass_by_value,
    reason = "public API: callers pass owned paths"
)]
pub fn cmd_sign(
    privatekey_path: PathBuf,
    pbo_path: PathBuf,
    signature_path: Option<PathBuf>,
    version: BISignVersion,
    force: bool,
) -> Result<(), Error> {
    let privatekey = BIPrivateKey::read(&mut open_input(&privatekey_path, "private key")?)
        .prepend_error("Failed to read private key:")?;
    let pbo = PBO::read(&mut open_input(&pbo_path, "PBO")?).prepend_error("Failed to read PBO:")?;

    let sig_path = if let Some(path) = signature_path {
        path
    } else {
        dayzmake_core::sign::signature_path(&pbo_path, privatekey.name())?
    };

    if !force && sig_path.exists() {
        return Err(error!(
            "Target file \"{}\" already exists. Use --force to overwrite.",
            sig_path.display()
        ));
    }
    let sig = privatekey.sign(&pbo, version)?;
    let mut file =
        AtomicTarget::create(&sig_path, true).prepend_error("Failed to open signature file:")?;
    sig.write(&mut file)
        .prepend_error("Failed to write signature:")?;
    file.commit()?;

    Ok(())
}

/// Verifies a signature for a pbo against a given public key.
///
/// If the signature path is not given it is inferred from the PBO path.
///
/// # Errors
///
/// Returns an error if the key, PBO or signature file cannot be read, or a hash doesn't match.
#[expect(
    clippy::needless_pass_by_value,
    reason = "public API: callers pass owned paths"
)]
pub fn cmd_verify(
    publickey_path: PathBuf,
    pbo_path: PathBuf,
    signature_path: Option<PathBuf>,
) -> Result<(), Error> {
    let publickey = BIPublicKey::read(&mut open_input(&publickey_path, "public key")?)
        .prepend_error("Failed to read public key:")?;
    let pbo = PBO::read(&mut open_input(&pbo_path, "PBO")?).prepend_error("Failed to read PBO:")?;

    let sig_path = if let Some(path) = signature_path {
        path
    } else {
        dayzmake_core::sign::signature_path(&pbo_path, publickey.name())?
    };

    let sig = BISign::read(&mut open_input(&sig_path, "signature")?)
        .prepend_error("Failed to read signature:")?;

    publickey.verify(&pbo, &sig)
}
