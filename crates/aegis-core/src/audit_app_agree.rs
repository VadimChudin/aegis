fn agree(local: &ModelDecision, cloud: &ModelDecision) -> Result<(), String> {
    if local.snapshot_id != cloud.snapshot_id || local.action != cloud.action || local.position_id != cloud.position_id
    {
        return Err("Models disagree on the action, snapshot or position; no action accepted".into());
    }
    if local.stop != cloud.stop || local.target != cloud.target || local.quantity_fraction != cloud.quantity_fraction {
        return Err("Models disagree on stop, target or reduction size; no action accepted".into());
    }
    Ok(())
}
