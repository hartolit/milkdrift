use std::io;

use milkdrift_persistence::{PersistenceError, StorageFailureClass};

const MAX_STORAGE_MESSAGE_BYTES: usize = 4_096;

pub(crate) fn database(error: redb::DatabaseError) -> PersistenceError {
    redb(error)
}

pub(crate) fn redb(error: impl Into<redb::Error>) -> PersistenceError {
    let error = error.into();
    let class = match &error {
        redb::Error::DatabaseAlreadyOpen => StorageFailureClass::OwnerBusy,
        redb::Error::Corrupted(_)
        | redb::Error::TableTypeMismatch { .. }
        | redb::Error::TableIsMultimap(_)
        | redb::Error::TableIsNotMultimap(_)
        | redb::Error::TypeDefinitionChanged { .. }
        | redb::Error::TableDoesNotExist(_)
        | redb::Error::TableExists(_) => StorageFailureClass::Corruption,
        redb::Error::UpgradeRequired(_) => StorageFailureClass::Migration,
        redb::Error::ValueTooLarge(_) => StorageFailureClass::ResourceExhausted,
        redb::Error::Io(_) | redb::Error::PreviousIo => StorageFailureClass::Unavailable,
        _ => StorageFailureClass::Internal,
    };
    storage(class, error.to_string())
}

pub(crate) fn io(error: io::Error) -> PersistenceError {
    storage(StorageFailureClass::Unavailable, error.to_string())
}

pub(crate) fn corruption(message: impl Into<String>) -> PersistenceError {
    storage(StorageFailureClass::Corruption, message.into())
}

pub(crate) fn internal(message: impl Into<String>) -> PersistenceError {
    storage(StorageFailureClass::Internal, message.into())
}

fn storage(class: StorageFailureClass, message: String) -> PersistenceError {
    let message =
        milkdrift_contracts::truncate_utf8(&message, MAX_STORAGE_MESSAGE_BYTES).to_owned();
    PersistenceError::Storage { class, message }
}
