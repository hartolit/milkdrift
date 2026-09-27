//! Historical installations preserve replay without consuming live setup capacity.
use super::*;

#[test]
fn removed_installations_turn_over_across_reopen_and_uncertain_removal_keeps_its_slot() -> Result {
    let root = tempfile::tempdir()?;
    let platform = Arc::new(Platform::default());
    let mut history = Vec::new();
    for generation in 0..6 {
        let store = Arc::new(RedbStore::open_with_config(
            RedbStoreConfig::new(root.path()).with_managed_limits(1, 1),
        )?);
        let manager = owner(store.clone(), platform.clone());
        manager.recover_startup()?;
        assert!(
            store
                .active_managed_installations(None, PageSize::new(1)?)?
                .is_empty()
        );
        let mut apply = request(
            &format!("apply-{generation}"),
            0,
            ManagedAction::Apply {
                recipe: reference('1')?,
            },
        )?;
        apply.installation = ManagedName::new(format!("installation-{generation}"))?;
        let accepted = manager.execute(&caller()?, &apply)?;
        let record = store
            .managed_installation(&apply.installation)?
            .ok_or("inventory absent")?;
        assert_eq!(
            store.active_managed_installations(None, PageSize::new(1)?)?,
            vec![record.clone()]
        );
        let mut excess = apply.clone();
        excess.command = ManagedName::new(format!("excess-{generation}"))?;
        excess.installation = ManagedName::new(format!("excess-{generation}"))?;
        assert!(manager.execute(&caller()?, &excess).is_err());
        assert!(store.managed_installation(&excess.installation)?.is_none());
        let mut remove = request(
            &format!("remove-{generation}"),
            record.version,
            ManagedAction::Remove {},
        )?;
        remove.installation = apply.installation.clone();
        platform.0.lock().map_err(|e| e.to_string())?.interrupt =
            Some((ManagedStep::RemoveConfiguration, false));
        manager.execute(&caller()?, &remove)?;
        assert!(manager.execute(&caller()?, &excess).is_err());
        let pending = store
            .managed_installation(&apply.installation)?
            .ok_or("pending absent")?;
        assert!(!pending.removed && pending.pending.is_some());
        drop(manager);
        drop(store);
        let store = Arc::new(RedbStore::open_with_config(
            RedbStoreConfig::new(root.path()).with_managed_limits(1, 1),
        )?);
        let manager = owner(store.clone(), platform.clone());
        manager.recover_startup()?;
        assert!(manager.execute(&caller()?, &excess).is_err());
        let mut recover = request(
            &format!("recover-{generation}"),
            pending.version,
            ManagedAction::Recover {},
        )?;
        recover.installation = apply.installation.clone();
        manager.execute(&caller()?, &recover)?;
        let removed = store
            .managed_installation(&apply.installation)?
            .ok_or("removed identity absent")?;
        assert!(removed.removed);
        assert!(
            store
                .active_managed_installations(None, PageSize::new(1)?)?
                .is_empty()
        );
        assert_eq!(manager.execute(&caller()?, &apply)?, accepted);
        let mut reused = apply.clone();
        reused.command = ManagedName::new(format!("reuse-{generation}"))?;
        reused.expected_version = removed.version;
        assert!(manager.execute(&caller()?, &reused).is_err());
        history.push((apply, accepted, removed));
    }
    let store = Arc::new(RedbStore::open_with_config(
        RedbStoreConfig::new(root.path()).with_managed_limits(1, 1),
    )?);
    let manager = owner(store.clone(), platform);
    manager.recover_startup()?;
    for (request, response, record) in history {
        assert_eq!(manager.execute(&caller()?, &request)?, response);
        assert_eq!(
            store.managed_installation(&request.installation)?,
            Some(record)
        );
        let mut conflict = request;
        conflict.action = ManagedAction::Remove {};
        assert!(manager.execute(&caller()?, &conflict).is_err());
    }
    assert_eq!(
        store.managed_installations(None, PageSize::new(16)?)?.len(),
        6
    );
    store.verify_managed_integrity()?;
    Ok(())
}
