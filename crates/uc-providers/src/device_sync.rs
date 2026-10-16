use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uc_accounts::AccountStore;
use uuid::Uuid;

use crate::{BillingPeriods, VisibleAccount};

pub const MAX_BYTES: usize = 1_048_576;
const MAX_FIELDS: usize = 4096;
const MAX_ACCOUNTS: usize = 512;
const PLAN_FRESHNESS: chrono::Duration = chrono::Duration::minutes(15);
const SETTINGS: &[&str] = &[
    "settingsRevision",
    "language",
    "theme",
    "density",
    "reduceAnimations",
    "timeFormat",
    "iconStyle",
    "showTaskbarStrip",
    "strip",
    "dynamicIsland",
    "island",
    "widget",
    "showTotalSpend",
    "showBenchmarkTab",
    "showResetsTab",
    "notifyCodexResets",
    "notifyClaudeResets",
    "resetsProvider",
    "usedBankedResets",
    "dashboardTab",
    "totalSpendPeriod",
    "totalSpendMetric",
    "tokenView",
    "tokenRingBy",
    "tokenChart",
    "tokenChartMetric",
    "tokenChartPeriod",
    "projectPeriod",
    "priceProvider",
    "priceTier",
    "priceCurrency",
    "displayMode",
    "resetDisplayMode",
    "alwaysShowPacing",
    "notifications",
    "customizeHintDismissed",
    "accountsHintDismissed",
    "automaticUpdateChecks",
    "automaticUpdateInstalls",
    "enabledProviders",
];
const LAYOUT: &[&str] = &[
    "placed",
    "providerOrder",
    "metricOrder",
    "pinned",
    "onDemand",
    "openProviders",
    "onDemandWhenEnabled",
    "offered",
    "revisions",
];

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Revision {
    pub counter: u64,
    pub device_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Field {
    pub revision: Revision,
    pub value: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountMetadata {
    pub id: String,
    pub provider: String,
    pub label: String,
    pub account_uuid: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountEntry {
    pub revision: Revision,
    pub account: Option<AccountMetadata>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BillingEvidence {
    pub provider: String,
    pub account_uuid: String,
    pub plan: Option<String>,
    pub ends_at: Option<DateTime<Utc>>,
    pub observed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BillingEntry {
    pub revision: Revision,
    pub evidence: BillingEvidence,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Envelope {
    pub version: u32,
    pub device_id: Uuid,
    pub fields: BTreeMap<String, Field>,
    pub accounts: BTreeMap<String, AccountEntry>,
    pub billing: BTreeMap<String, BillingEntry>,
}

impl Envelope {
    pub fn from_slice(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err("Sync payload exceeds 1 MiB".into());
        }
        let envelope: Self = serde_json::from_slice(bytes).map_err(|_| "Invalid sync payload")?;
        envelope.validate(Utc::now())?;
        Ok(envelope)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let bytes = serde_json::to_vec(self).map_err(|_| "Cannot encode sync payload")?;
        if bytes.len() > MAX_BYTES {
            return Err("Sync payload exceeds 1 MiB".into());
        }
        Ok(bytes)
    }

    fn validate(&self, now: DateTime<Utc>) -> Result<(), String> {
        if self.version != 1
            || self.device_id.is_nil()
            || self.fields.len() > MAX_FIELDS
            || self.accounts.len() > MAX_ACCOUNTS
            || self.billing.len() > MAX_ACCOUNTS
        {
            return Err("Unsupported or oversized sync document".into());
        }
        self.to_bytes()?;
        for (path, field) in &self.fields {
            valid_revision(&field.revision)?;
            if !allowed_path(path)
                || field
                    .value
                    .as_ref()
                    .is_some_and(|value| !safe_value(value, 0))
                || (path.starts_with("settings/enabledProviders/")
                    && field
                        .value
                        .as_ref()
                        .is_some_and(|value| !value.is_boolean()))
                || (path == "settings/settingsRevision"
                    && !field.value.as_ref().is_some_and(|value| {
                        value.as_u64().is_some_and(|value| value <= u32::MAX as u64)
                    }))
            {
                return Err("Sync contains a non-portable setting".into());
            }
        }
        for (id, entry) in &self.accounts {
            valid_revision(&entry.revision)?;
            if !valid_account_id(id) {
                return Err("Invalid sync account identity".into());
            }
            if let Some(account) = &entry.account
                && (account.id != *id || !valid_metadata(account))
            {
                return Err("Invalid sync account metadata".into());
            }
        }
        for (key, entry) in &self.billing {
            valid_revision(&entry.revision)?;
            let evidence = &entry.evidence;
            if *key != billing_key(evidence)
                || !valid_uuid(&evidence.account_uuid)
                || !matches!(evidence.provider.as_str(), "claude" | "codex")
                || evidence.observed_at > now
                || match (&evidence.plan, evidence.ends_at) {
                    (None, None) => false,
                    (Some(plan), Some(end)) => {
                        !valid_paid_plan(&evidence.provider, plan) || end <= evidence.observed_at
                    }
                    _ => true,
                }
            {
                return Err("Invalid sync billing evidence".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct LocalAccount {
    pub metadata: AccountMetadata,
    pub plan: Option<String>,
    pub plan_checked_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct LocalState {
    pub settings: Value,
    pub layout: Value,
    pub accounts: Vec<LocalAccount>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportReport {
    pub changed: bool,
    pub revision: u64,
    pub settings_changed: bool,
    pub layout_changed: bool,
    pub billing_applied: usize,
    pub billing_pending: usize,
    pub pending_accounts: usize,
}

pub struct ImportResult {
    pub report: ImportReport,
    pub settings: Value,
    pub layout: Value,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct State {
    envelope: Envelope,
    clock: u64,
    revision: u64,
    #[serde(default)]
    billing_updates: BTreeMap<String, u64>,
    local_fields: BTreeMap<String, Value>,
    local_accounts: BTreeMap<String, AccountMetadata>,
}

impl State {
    fn new() -> Self {
        Self {
            envelope: Envelope {
                version: 1,
                device_id: Uuid::new_v4(),
                fields: BTreeMap::new(),
                accounts: BTreeMap::new(),
                billing: BTreeMap::new(),
            },
            clock: 0,
            revision: 0,
            billing_updates: BTreeMap::new(),
            local_fields: BTreeMap::new(),
            local_accounts: BTreeMap::new(),
        }
    }

    fn next(&mut self) -> Result<Revision, String> {
        self.clock = self.clock.checked_add(1).ok_or("Sync revision exhausted")?;
        Ok(Revision {
            counter: self.clock,
            device_id: self.envelope.device_id,
        })
    }

    fn observe(
        &mut self,
        local: &LocalState,
        billing: &BillingPeriods,
        now: DateTime<Utc>,
    ) -> Result<(), String> {
        let fields = portable_fields(&local.settings, &local.layout)?;
        let keys: BTreeSet<_> = fields
            .keys()
            .chain(self.local_fields.keys())
            .cloned()
            .collect();
        for path in keys {
            if path == "settings/settingsRevision"
                && self.envelope.fields.get(&path).is_some_and(|entry| {
                    entry.value.as_ref().and_then(Value::as_u64)
                        > fields.get(&path).and_then(Value::as_u64)
                })
            {
                continue;
            }
            if path.starts_with("settings/enabledProviders/") && !fields.contains_key(&path) {
                continue;
            }
            if path.starts_with("settings/enabledProviders/")
                && !self.local_fields.contains_key(&path)
                && self.envelope.fields.contains_key(&path)
            {
                continue;
            }
            if fields.get(&path) != self.local_fields.get(&path) {
                let revision = self.next()?;
                self.envelope.fields.insert(
                    path.clone(),
                    Field {
                        revision,
                        value: fields.get(&path).cloned(),
                    },
                );
            }
        }
        self.local_fields = fields;
        for local in &local.accounts {
            let account = &local.metadata;
            if !valid_metadata(account) {
                return Err("Invalid local account identity".into());
            }
            if self
                .envelope
                .accounts
                .get(&account.id)
                .is_some_and(|entry| entry.account.is_none())
            {
                self.local_accounts
                    .insert(account.id.clone(), account.clone());
                continue;
            }
            if self.local_accounts.get(&account.id) != Some(account) {
                let revision = self.next()?;
                self.envelope.accounts.insert(
                    account.id.clone(),
                    AccountEntry {
                        revision,
                        account: Some(account.clone()),
                    },
                );
                self.local_accounts
                    .insert(account.id.clone(), account.clone());
            }
        }
        for evidence in billing
            .sync_evidence(now)
            .map_err(|_| "Cannot read billing evidence")?
        {
            let key = billing_key(&evidence);
            if self
                .envelope
                .billing
                .get(&key)
                .is_none_or(|entry| evidence_order(&evidence, &entry.evidence).is_gt())
            {
                let revision = self.next()?;
                self.envelope
                    .billing
                    .insert(key, BillingEntry { revision, evidence });
            }
        }
        self.envelope.validate(now)
    }
}

pub struct DeviceSyncStore {
    root: PathBuf,
}

impl DeviceSyncStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn default_store() -> Self {
        Self::new(uc_core::paths::config_dir().join("device-sync"))
    }

    pub fn export(
        &self,
        local: &LocalState,
        billing: &BillingPeriods,
        now: DateTime<Utc>,
    ) -> Result<Envelope, String> {
        let _guard = self.lock()?;
        let mut state = self.load()?;
        state.observe(local, billing, now)?;
        self.save(&state)?;
        Ok(state.envelope)
    }

    pub fn import(
        &self,
        incoming: &Envelope,
        local: &LocalState,
        billing: &BillingPeriods,
        now: DateTime<Utc>,
    ) -> Result<ImportResult, String> {
        self.import_with(incoming, local, billing, now, |_, _| Ok(()))
    }

    fn import_with(
        &self,
        incoming: &Envelope,
        local: &LocalState,
        billing: &BillingPeriods,
        now: DateTime<Utc>,
        persist: impl FnOnce(&Value, &Value) -> Result<(), String>,
    ) -> Result<ImportResult, String> {
        incoming.validate(now)?;
        let _guard = self.lock()?;
        let mut state = self.load()?;
        state.observe(local, billing, now)?;
        let previous = state.envelope.clone();
        for (key, value) in &incoming.fields {
            state.clock = state.clock.max(value.revision.counter);
            if key == "settings/settingsRevision"
                && state.envelope.fields.get(key).is_some_and(|current| {
                    current.value.as_ref().and_then(Value::as_u64)
                        > value.value.as_ref().and_then(Value::as_u64)
                })
            {
                continue;
            }
            if state.envelope.fields.get(key).is_none_or(|current| {
                value.revision > current.revision
                    || (key == "settings/settingsRevision"
                        && value.value.as_ref().and_then(Value::as_u64)
                            > current.value.as_ref().and_then(Value::as_u64))
            }) {
                state.envelope.fields.insert(key.clone(), value.clone());
            }
        }
        for (key, value) in &incoming.accounts {
            state.clock = state.clock.max(value.revision.counter);
            if state
                .envelope
                .accounts
                .get(key)
                .is_none_or(|current| value.revision > current.revision)
            {
                state.envelope.accounts.insert(key.clone(), value.clone());
            }
        }
        for (key, value) in &incoming.billing {
            state.clock = state.clock.max(value.revision.counter);
            if state.envelope.billing.get(key).is_none_or(|current| {
                evidence_order(&value.evidence, &current.evidence).is_gt()
                    || (value.evidence == current.evidence && value.revision > current.revision)
            }) {
                state.envelope.billing.insert(key.clone(), value.clone());
            }
        }
        state.envelope.validate(now)?;
        let (settings, layout) =
            render_documents(&state.envelope.fields, &local.settings, &local.layout)?;
        let mut report = ImportReport {
            changed: state.envelope != previous,
            settings_changed: settings != local.settings,
            layout_changed: layout != local.layout,
            pending_accounts: pending(&state, local).len(),
            ..Default::default()
        };
        let before_billing = billing
            .sync_evidence(now)
            .map_err(|_| "Cannot read billing evidence")?;
        for entry in state.envelope.billing.values() {
            let evidence = &entry.evidence;
            let matched = local.accounts.iter().find(|account| {
                account.metadata.provider == evidence.provider
                    && account.metadata.account_uuid == evidence.account_uuid
            });
            let Some(account) = matched else {
                report.billing_pending += 1;
                continue;
            };
            if account
                .plan_checked_at
                .is_none_or(|at| at > now || now - at > PLAN_FRESHNESS)
            {
                report.billing_pending += 1;
                continue;
            }
            if let (Some(plan), Some(end)) = (&evidence.plan, evidence.ends_at) {
                if account.plan.as_ref() != Some(plan) || end <= now {
                    report.billing_pending += 1;
                    continue;
                }
                billing
                    .save_from(
                        &evidence.provider,
                        &evidence.account_uuid,
                        plan,
                        Some(end),
                        evidence.observed_at,
                        &format!("device:{}", entry.revision.device_id),
                    )
                    .map_err(|_| "Cannot merge billing evidence")?;
            } else {
                billing
                    .invalidate_for_at(
                        &evidence.provider,
                        &evidence.account_uuid,
                        evidence.observed_at,
                    )
                    .map_err(|_| "Cannot merge billing revocation")?;
            }
        }
        let after_billing = billing
            .sync_evidence(now)
            .map_err(|_| "Cannot verify merged billing")?;
        report.billing_applied = after_billing
            .iter()
            .filter(|entry| !before_billing.contains(entry))
            .count();
        report.changed |=
            report.settings_changed || report.layout_changed || report.billing_applied > 0;
        if report.changed {
            state.revision = state
                .revision
                .checked_add(1)
                .ok_or("Sync revision exhausted")?;
        }
        state.billing_updates.retain(|id, _| {
            local
                .accounts
                .iter()
                .any(|account| account.metadata.id == *id)
        });
        for evidence in after_billing
            .iter()
            .filter(|entry| !before_billing.contains(entry))
        {
            for account in &local.accounts {
                if account.metadata.provider == evidence.provider
                    && account.metadata.account_uuid == evidence.account_uuid
                {
                    state
                        .billing_updates
                        .insert(account.metadata.id.clone(), state.revision);
                }
            }
        }
        if state.billing_updates.len() > MAX_ACCOUNTS {
            return Err("Too many billing update identities".into());
        }
        report.revision = state.revision;
        persist(&settings, &layout)?;
        state.local_fields = portable_fields(&settings, &layout)?;
        self.save(&state)?;
        Ok(ImportResult {
            report,
            settings,
            layout,
        })
    }

    pub fn revision(&self) -> Result<u64, String> {
        let _guard = self.lock()?;
        Ok(self.load()?.revision)
    }

    pub fn billing_updates_since(&self, revision: u64) -> Result<Vec<String>, String> {
        let _guard = self.lock()?;
        Ok(self
            .load()?
            .billing_updates
            .into_iter()
            .filter_map(|(id, updated)| (updated > revision).then_some(id))
            .collect())
    }

    pub fn pending(&self, local: &LocalState) -> Result<Vec<AccountMetadata>, String> {
        let _guard = self.lock()?;
        Ok(pending(&self.load()?, local))
    }

    pub fn remove_account(&self, id: &str) -> Result<(), String> {
        if !valid_account_id(id) {
            return Err("Invalid sync account identity".into());
        }
        let _guard = self.lock()?;
        let mut state = self.load()?;
        let revision = state.next()?;
        state.envelope.accounts.insert(
            id.into(),
            AccountEntry {
                revision,
                account: None,
            },
        );
        self.save(&state)
    }

    pub fn restore_account(&self, account: AccountMetadata) -> Result<(), String> {
        if !valid_metadata(&account) {
            return Err("Invalid local account identity".into());
        }
        let _guard = self.lock()?;
        let mut state = self.load()?;
        let revision = state.next()?;
        state
            .local_accounts
            .insert(account.id.clone(), account.clone());
        state.envelope.accounts.insert(
            account.id.clone(),
            AccountEntry {
                revision,
                account: Some(account),
            },
        );
        self.save(&state)
    }

    fn lock(&self) -> Result<File, String> {
        reject_link(&self.root)?;
        std::fs::create_dir_all(&self.root).map_err(|_| "Cannot create sync storage")?;
        set_private(&self.root, true)?;
        let path = self.root.join("state.lock");
        reject_link(&path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path).map_err(|_| "Cannot open sync lock")?;
        file.lock().map_err(|_| "Cannot lock sync storage")?;
        Ok(file)
    }

    fn load(&self) -> Result<State, String> {
        let path = self.root.join("state.json");
        reject_link(&path)?;
        match read_bounded(&path) {
            Ok(Some(bytes)) => {
                let state: State = serde_json::from_slice(&bytes)
                    .map_err(|_| "Invalid sync state; existing file was preserved")?;
                state.envelope.validate(Utc::now())?;
                if state.billing_updates.len() > MAX_ACCOUNTS
                    || state.billing_updates.iter().any(|(id, revision)| {
                        !valid_account_id(id) || *revision == 0 || *revision > state.revision
                    })
                {
                    return Err("Invalid billing update state".into());
                }
                if state
                    .envelope
                    .fields
                    .values()
                    .any(|entry| entry.revision.counter > state.clock)
                    || state
                        .envelope
                        .accounts
                        .values()
                        .any(|entry| entry.revision.counter > state.clock)
                    || state
                        .envelope
                        .billing
                        .values()
                        .any(|entry| entry.revision.counter > state.clock)
                {
                    return Err("Invalid sync revision state".into());
                }
                Ok(state)
            }
            Ok(None) => Ok(State::new()),
            Err(error) => Err(error),
        }
    }

    fn save(&self, state: &State) -> Result<(), String> {
        let bytes = serde_json::to_vec(state).map_err(|_| "Cannot encode sync state")?;
        if bytes.len() > MAX_BYTES {
            return Err("Sync state exceeds 1 MiB".into());
        }
        let path = self.root.join("state.json");
        reject_link(&path)?;
        uc_core::paths::write_atomic(&path, &bytes).map_err(|_| "Cannot save sync state")?;
        set_private(&path, false)
    }
}

pub fn export_state() -> Result<Envelope, String> {
    let _guard = uc_core::paths::lock_documents(&uc_core::paths::config_dir())
        .map_err(|_| "Cannot lock local preferences")?;
    DeviceSyncStore::default_store().export(
        &read_local()?,
        &BillingPeriods::default_store(),
        Utc::now(),
    )
}

pub fn import_state(envelope: Envelope) -> Result<ImportReport, String> {
    let _guard = uc_core::paths::lock_documents(&uc_core::paths::config_dir())
        .map_err(|_| "Cannot lock local preferences")?;
    let local = read_local()?;
    let result = DeviceSyncStore::default_store().import_with(
        &envelope,
        &local,
        &BillingPeriods::default_store(),
        Utc::now(),
        |settings, layout| {
            for (name, value, old) in [
                ("settings.json", settings, &local.settings),
                ("layout.json", layout, &local.layout),
            ] {
                if value != old {
                    let path = uc_core::paths::config_dir().join(name);
                    if read_document(&path)? != *old {
                        return Err("Local preferences changed during sync; retry the merge".into());
                    }
                    let bytes = serde_json::to_vec_pretty(value)
                        .map_err(|_| "Cannot encode synced preferences")?;
                    uc_core::paths::write_atomic(&path, &bytes)
                        .map_err(|_| "Cannot save synced preferences")?;
                }
            }
            Ok(())
        },
    )?;
    Ok(result.report)
}

pub fn status_revision() -> Result<u64, String> {
    DeviceSyncStore::default_store().revision()
}

pub fn billing_updates_since(revision: u64) -> Result<Vec<String>, String> {
    DeviceSyncStore::default_store().billing_updates_since(revision)
}

pub fn pending_accounts() -> Result<Vec<AccountMetadata>, String> {
    DeviceSyncStore::default_store().pending(&read_local()?)
}

pub fn remove_synced_account(id: &str) -> Result<(), String> {
    DeviceSyncStore::default_store().remove_account(id)
}

pub fn restore_synced_account(id: &str) -> Result<(), String> {
    let account = read_local()?
        .accounts
        .into_iter()
        .find(|account| account.metadata.id == id)
        .ok_or("The account is not connected on this device")?;
    DeviceSyncStore::default_store().restore_account(account.metadata)
}

pub fn save_document_changes(
    name: &str,
    previous: Option<&Value>,
    next: &Value,
) -> Result<(), String> {
    save_document_changes_at(&uc_core::paths::config_dir(), name, previous, next)
}

pub fn save_document_changes_at(
    root: &Path,
    name: &str,
    previous: Option<&Value>,
    next: &Value,
) -> Result<(), String> {
    if !matches!(name, "settings" | "layout") || !next.is_object() {
        return Err("Invalid preference document".into());
    }
    if serde_json::to_vec(next)
        .map_err(|_| "Cannot encode preferences")?
        .len()
        > MAX_BYTES
    {
        return Err("Preference document exceeds 1 MiB".into());
    }
    let _guard =
        uc_core::paths::lock_documents(root).map_err(|_| "Cannot lock local preferences")?;
    let path = root.join(format!("{name}.json"));
    let mut current = read_document(&path)?;
    if let Some(previous) = previous {
        if !previous.is_object() {
            return Err("Invalid previous preference document".into());
        }
        if serde_json::to_vec(previous)
            .map_err(|_| "Cannot encode previous preferences")?
            .len()
            > MAX_BYTES
        {
            return Err("Previous preference document exceeds 1 MiB".into());
        }
        merge_delta(&mut current, previous, next);
    } else {
        current = next.clone();
    }
    let bytes =
        serde_json::to_vec_pretty(&current).map_err(|_| "Cannot encode merged preferences")?;
    if bytes.len() > MAX_BYTES {
        return Err("Preference document exceeds 1 MiB".into());
    }
    uc_core::paths::write_atomic(&path, &bytes).map_err(|_| "Cannot save merged preferences".into())
}

fn merge_delta(current: &mut Value, previous: &Value, next: &Value) {
    if previous == next {
        return;
    }
    if let (Some(previous), Some(next), Some(current)) = (
        previous.as_object(),
        next.as_object(),
        current.as_object_mut(),
    ) {
        for key in previous.keys().chain(next.keys()).collect::<BTreeSet<_>>() {
            match (previous.get(key), next.get(key)) {
                (Some(before), Some(after)) if before != after => merge_delta(
                    current.entry(key.clone()).or_insert(Value::Null),
                    before,
                    after,
                ),
                (None, Some(after)) => {
                    current.insert(key.clone(), after.clone());
                }
                (Some(_), None) => {
                    current.remove(key);
                }
                _ => {}
            }
        }
    } else {
        *current = next.clone();
    }
}

fn read_local() -> Result<LocalState, String> {
    let store = Arc::new(AccountStore::default_store());
    let records = store.list().map_err(|_| "Cannot read local accounts")?;
    let cli = crate::cli_accounts();
    let cache = read_document(&uc_core::paths::cache_dir().join("provider-snapshots.v1.json"))?;
    let mut accounts = Vec::new();
    for visible in crate::visible_accounts(&records, &cli) {
        let (id, provider, label) = match visible {
            VisibleAccount::Stored(record) => (
                record.id.clone(),
                record.provider.clone(),
                record.label.clone(),
            ),
            VisibleAccount::Cli(account) => (
                account.id.clone(),
                account.kind.cli().into(),
                account
                    .email
                    .clone()
                    .unwrap_or_else(|| account.kind.cli().into()),
            ),
        };
        let Ok(account_uuid) = crate::accounts::billing_account_uuid(store.clone(), &cli, &id)
        else {
            continue;
        };
        let snapshot = &cache["snapshots"][&id];
        accounts.push(LocalAccount {
            metadata: AccountMetadata {
                id,
                provider,
                label,
                account_uuid,
            },
            plan: snapshot["plan"].as_str().map(str::to_owned),
            plan_checked_at: snapshot["planCheckedAt"]
                .as_str()
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|at| at.with_timezone(&Utc)),
        });
    }
    Ok(LocalState {
        settings: read_document(&uc_core::paths::config_dir().join("settings.json"))?,
        layout: read_document(&uc_core::paths::config_dir().join("layout.json"))?,
        accounts,
    })
}

fn pending(state: &State, local: &LocalState) -> Vec<AccountMetadata> {
    state
        .envelope
        .accounts
        .values()
        .filter_map(|entry| entry.account.as_ref())
        .filter(|account| {
            !local.accounts.iter().any(|local| {
                local.metadata.id == account.id
                    && local.metadata.account_uuid == account.account_uuid
            })
        })
        .cloned()
        .collect()
}

fn billing_key(evidence: &BillingEvidence) -> String {
    format!("{}@{}", evidence.provider, evidence.account_uuid)
}

fn evidence_order(left: &BillingEvidence, right: &BillingEvidence) -> std::cmp::Ordering {
    (left.observed_at, left.ends_at.is_none()).cmp(&(right.observed_at, right.ends_at.is_none()))
}

fn valid_revision(revision: &Revision) -> Result<(), String> {
    if revision.counter == 0 || revision.counter == u64::MAX || revision.device_id.is_nil() {
        Err("Invalid sync source revision".into())
    } else {
        Ok(())
    }
}

fn valid_uuid(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| !id.is_nil() && id.to_string() == value)
}

fn valid_account_id(id: &str) -> bool {
    id.split_once('@').is_some_and(|(provider, hash)| {
        matches!(provider, "claude" | "codex")
            && hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn valid_metadata(account: &AccountMetadata) -> bool {
    valid_account_id(&account.id)
        && account.id.starts_with(&format!("{}@", account.provider))
        && valid_uuid(&account.account_uuid)
        && !account.label.is_empty()
        && account.label.len() <= 256
        && !account.label.chars().any(char::is_control)
}

fn valid_paid_plan(provider: &str, plan: &str) -> bool {
    if plan.len() > 64 || plan.chars().any(char::is_control) {
        return false;
    }
    match provider {
        "claude" => {
            let mut words = plan.split_whitespace();
            matches!(words.next(), Some("Pro" | "Max" | "Team"))
                && words.next().is_none_or(|word| {
                    word.strip_suffix('x')
                        .is_some_and(|value| value.parse::<u32>().is_ok())
                })
                && words.next().is_none()
        }
        "codex" => {
            !plan.trim().is_empty()
                && !matches!(plan.to_ascii_lowercase().as_str(), "free" | "unknown")
        }
        _ => false,
    }
}

fn safe_value(value: &Value, depth: usize) -> bool {
    if depth > 3 {
        return false;
    }
    match value {
        Value::Null => false,
        Value::Bool(_) | Value::Number(_) => true,
        Value::String(value) => value.len() <= 256 && !value.chars().any(char::is_control),
        Value::Array(values) => {
            values.len() <= MAX_FIELDS && values.iter().all(|value| safe_value(value, depth + 1))
        }
        Value::Object(_) => false,
    }
}

fn allowed_path(path: &str) -> bool {
    if path.len() > 512 {
        return false;
    }
    let parts: Vec<_> = path.split('/').collect();
    match parts.as_slice() {
        ["settings", key] => {
            SETTINGS.contains(key)
                && !matches!(
                    *key,
                    "strip" | "island" | "widget" | "notifications" | "enabledProviders"
                )
        }
        ["settings", "enabledProviders", id] => valid_provider_id(id),
        ["settings", "notifications", key] => {
            matches!(*key, "almostOut" | "cuttingItClose" | "willRunOut")
        }
        ["settings", "strip", key] => matches!(*key, "content" | "metrics" | "values"),
        ["settings", surface, key] if matches!(*surface, "island" | "widget") => matches!(
            *key,
            "content"
                | "metrics"
                | "showAccount"
                | "showPlan"
                | "showResets"
                | "showProblems"
                | "tabs"
                | "upcomingLimit"
                | "resetsProvider"
                | "style"
                | "wings"
                | "expandOnHover"
                | "alerts"
                | "layout"
        ),
        ["settings", surface, "resetParts", key] if matches!(*surface, "island" | "widget") => {
            matches!(
                *key,
                "next" | "latest" | "chances" | "wait" | "calendar" | "rhythm"
            )
        }
        ["layout", key] => LAYOUT.contains(key) && *key != "metricOrder",
        ["layout", "metricOrder", id] => {
            !id.is_empty()
                && id.len() <= 128
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'@' | b'-' | b'_'))
        }
        _ => false,
    }
}

fn portable_fields(settings: &Value, layout: &Value) -> Result<BTreeMap<String, Value>, String> {
    let settings = normalized_settings(settings);
    let settings = &settings;
    let mut output = BTreeMap::new();
    for (prefix, value, keys) in [("settings", settings, SETTINGS), ("layout", layout, LAYOUT)] {
        let Some(object) = value.as_object() else {
            return Err("Invalid local preferences; existing file was preserved".into());
        };
        for (key, value) in object {
            if prefix == "settings" && key == "enabledProviders" {
                continue;
            }
            if keys.contains(&key.as_str()) {
                flatten(&format!("{prefix}/{key}"), value, &mut output)?;
            }
        }
    }
    for id in known_providers(settings) {
        let enabled = settings["enabledProviders"]
            .as_array()
            .is_none_or(|values| values.iter().any(|value| value.as_str() == Some(&id)));
        output.insert(format!("settings/enabledProviders/{id}"), json!(enabled));
    }
    Ok(output)
}

fn normalized_settings(settings: &Value) -> Value {
    let mut settings = settings.clone();
    let Some(object) = settings.as_object_mut() else {
        return settings;
    };
    let revision = object
        .get("settingsRevision")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    object.insert("settingsRevision".into(), json!(revision.max(1)));
    if let Some(island) = object.get_mut("island").and_then(Value::as_object_mut) {
        if revision < 1 {
            island.insert("showPlan".into(), json!(true));
        }
        if !island.contains_key("tabs")
            && (island.contains_key("view") || island.contains_key("sections"))
        {
            let views = ["quota", "resets", "upcoming"];
            let mut tabs: Vec<_> = views
                .iter()
                .copied()
                .filter(|view| {
                    island
                        .get("sections")
                        .is_some_and(|value| value[view] == true)
                })
                .collect();
            if island.get("layout").and_then(Value::as_str) == Some("combined") {
                if tabs.is_empty() {
                    tabs.push("quota");
                }
            } else {
                let view = island
                    .get("view")
                    .and_then(Value::as_str)
                    .filter(|view| views.contains(view))
                    .unwrap_or("quota");
                tabs.retain(|tab| *tab != view);
                tabs.insert(0, view);
            }
            island.insert("tabs".into(), json!(tabs));
        }
        island.remove("view");
        island.remove("sections");
    }
    settings
}

fn valid_provider_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'@' | b'-' | b'_'))
}

fn known_providers(settings: &Value) -> Vec<String> {
    settings["knownProviders"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|id| valid_provider_id(id))
        .map(str::to_owned)
        .collect()
}

fn flatten(path: &str, value: &Value, output: &mut BTreeMap<String, Value>) -> Result<(), String> {
    if let Some(object) = value.as_object() {
        for (key, value) in object {
            if key.contains('/') || key.contains('~') {
                return Err("Invalid portable preference key".into());
            }
            flatten(&format!("{path}/{key}"), value, output)?;
        }
    } else if allowed_path(path) && safe_value(value, 0) {
        output.insert(path.into(), value.clone());
    } else {
        return Err("Invalid portable preference value".into());
    }
    Ok(())
}

fn render_documents(
    fields: &BTreeMap<String, Field>,
    settings: &Value,
    layout: &Value,
) -> Result<(Value, Value), String> {
    let mut settings = settings.clone();
    let mut layout = layout.clone();
    if layout.get("version").is_none() && fields.keys().any(|key| key.starts_with("layout/")) {
        layout["version"] = json!(1);
    }
    for (path, field) in fields {
        if path.starts_with("settings/enabledProviders/") {
            continue;
        }
        let (kind, path) = path.split_once('/').ok_or("Invalid portable field")?;
        let document = if kind == "settings" {
            &mut settings
        } else {
            &mut layout
        };
        let parts: Vec<_> = path.split('/').collect();
        put_field(document, &parts, field.value.as_ref())?;
    }
    let known = known_providers(&settings);
    if !known.is_empty() {
        let enabled: Vec<_> = known
            .into_iter()
            .filter(|id| {
                fields
                    .get(&format!("settings/enabledProviders/{id}"))
                    .and_then(|entry| entry.value.as_ref())
                    .and_then(Value::as_bool)
                    .unwrap_or_else(|| {
                        settings["enabledProviders"]
                            .as_array()
                            .is_none_or(|values| {
                                values.iter().any(|value| value.as_str() == Some(id))
                            })
                    })
            })
            .collect();
        settings["enabledProviders"] = json!(enabled);
    }
    Ok((settings, layout))
}

fn put_field(document: &mut Value, parts: &[&str], value: Option<&Value>) -> Result<(), String> {
    let object = document
        .as_object_mut()
        .ok_or("Invalid preference object")?;
    if parts.len() == 1 {
        match value {
            Some(value) => {
                object.insert(parts[0].into(), value.clone());
            }
            None => {
                object.remove(parts[0]);
            }
        }
    } else {
        if value.is_none() && !object.contains_key(parts[0]) {
            return Ok(());
        }
        put_field(
            object.entry(parts[0]).or_insert_with(|| json!({})),
            &parts[1..],
            value,
        )?;
    }
    Ok(())
}

fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, String> {
    use std::io::Read;
    reject_link(path)?;
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Cannot read sync document".into()),
    };
    let mut bytes = Vec::new();
    file.take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read sync document")?;
    if bytes.len() > MAX_BYTES {
        return Err("Sync document exceeds 1 MiB".into());
    }
    Ok(Some(bytes))
}

fn read_document(path: &Path) -> Result<Value, String> {
    read_bounded(path)?
        .map(|bytes| {
            serde_json::from_slice(&bytes)
                .map_err(|_| "Invalid local document; existing file was preserved".into())
        })
        .unwrap_or_else(|| Ok(json!({})))
}

fn reject_link(path: &Path) -> Result<(), String> {
    for ancestor in path.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err("Unsafe sync storage path".into());
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if metadata.file_attributes() & 0x400 != 0 {
                        return Err("Unsafe sync storage path".into());
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Cannot inspect sync storage".into()),
        }
    }
    Ok(())
}

fn set_private(path: &Path, directory: bool) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            path,
            std::fs::Permissions::from_mode(if directory { 0o700 } else { 0o600 }),
        )
        .map_err(|_| "Cannot protect sync storage")?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, directory);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    const UUID: &str = "00000000-0000-4000-8000-000000000001";
    const OTHER_UUID: &str = "00000000-0000-4000-8000-000000000002";

    fn local(now: DateTime<Utc>) -> LocalState {
        LocalState {
            settings: json!({"theme":"light", "language":"en", "knownProviders":["machine-local"], "notifications":{"almostOut":true,"willRunOut":false}}),
            layout: json!({"version":1,"providerOrder":[],"pinned":[]}),
            accounts: vec![LocalAccount {
                metadata: AccountMetadata {
                    id: format!("codex@{}", "a".repeat(64)),
                    provider: "codex".into(),
                    label: "Account".into(),
                    account_uuid: UUID.into(),
                },
                plan: Some("Pro 5x".into()),
                plan_checked_at: Some(now),
            }],
        }
    }

    fn stores(root: &Path) -> (DeviceSyncStore, BillingPeriods) {
        (
            DeviceSyncStore::new(root.join("sync")),
            BillingPeriods::new(root.join("billing")),
        )
    }

    fn use_result(local: &mut LocalState, result: &ImportResult) {
        local.settings = result.settings.clone();
        local.layout = result.layout.clone();
    }

    #[test]
    fn concurrent_fields_merge_and_reimports_are_idempotent() {
        let now = Utc::now();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let (a_store, a_billing) = stores(a.path());
        let (b_store, b_billing) = stores(b.path());
        let mut a_local = local(now);
        let mut b_local = local(now);
        let initial = a_store.export(&a_local, &a_billing, now).unwrap();
        let result = b_store.import(&initial, &b_local, &b_billing, now).unwrap();
        use_result(&mut b_local, &result);
        a_local.settings["language"] = json!("vi");
        b_local.settings["theme"] = json!("dark");
        let a_out = a_store.export(&a_local, &a_billing, now).unwrap();
        let b_out = b_store.export(&b_local, &b_billing, now).unwrap();
        let result = a_store.import(&b_out, &a_local, &a_billing, now).unwrap();
        use_result(&mut a_local, &result);
        let result = b_store.import(&a_out, &b_local, &b_billing, now).unwrap();
        use_result(&mut b_local, &result);
        assert_eq!(a_local.settings, b_local.settings);
        assert_eq!(a_local.settings["theme"], "dark");
        assert_eq!(a_local.settings["language"], "vi");
        assert_eq!(a_local.settings["knownProviders"], json!(["machine-local"]));
        let revision = b_store.revision().unwrap();
        let replay = b_store.import(&a_out, &b_local, &b_billing, now).unwrap();
        assert!(!replay.report.changed);
        assert_eq!(replay.report.revision, revision);
    }

    #[test]
    fn same_field_concurrent_changes_have_a_stable_device_tiebreak() {
        let now = Utc::now();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let (a_store, a_billing) = stores(a.path());
        let (b_store, b_billing) = stores(b.path());
        let mut a_local = local(now);
        let mut b_local = local(now);
        let a_seed = a_store.export(&a_local, &a_billing, now).unwrap();
        let result = b_store.import(&a_seed, &b_local, &b_billing, now).unwrap();
        use_result(&mut b_local, &result);
        a_local.settings["theme"] = json!("dark");
        b_local.settings["theme"] = json!("system");
        let a_out = a_store.export(&a_local, &a_billing, now).unwrap();
        let b_out = b_store.export(&b_local, &b_billing, now).unwrap();
        let a_result = a_store.import(&b_out, &a_local, &a_billing, now).unwrap();
        let b_result = b_store.import(&a_out, &b_local, &b_billing, now).unwrap();
        assert_eq!(a_result.settings, b_result.settings);
    }

    #[test]
    fn offline_positive_cannot_resurrect_a_canceled_subscription() {
        let now = Utc::now();
        let first = now - Duration::minutes(10);
        let canceled = now - Duration::minutes(5);
        let end = now + Duration::days(20);
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let (a_store, a_billing) = stores(a.path());
        let (b_store, b_billing) = stores(b.path());
        let mut b_local = local(now);
        a_billing
            .save_from("codex", UUID, "Pro 5x", Some(end), first, "browser")
            .unwrap();
        let old = a_store.export(&local(now), &a_billing, now).unwrap();
        let result = b_store.import(&old, &b_local, &b_billing, now).unwrap();
        use_result(&mut b_local, &result);
        assert!(b_billing.peek_for("codex", UUID, now).is_some());
        a_billing
            .invalidate_for_at("codex", UUID, canceled)
            .unwrap();
        let revoked = a_store.export(&local(now), &a_billing, now).unwrap();
        let result = b_store.import(&revoked, &b_local, &b_billing, now).unwrap();
        use_result(&mut b_local, &result);
        assert!(b_billing.peek_for("codex", UUID, now).is_none());
        b_store.import(&old, &b_local, &b_billing, now).unwrap();
        b_billing
            .save_from("codex", UUID, "Pro 5x", Some(end), first, "browser")
            .unwrap();
        assert!(b_billing.peek_for("codex", UUID, now).is_none());
        let final_state = b_store.export(&b_local, &b_billing, now).unwrap();
        assert!(
            final_state
                .billing
                .values()
                .all(|entry| entry.evidence.ends_at.is_none())
        );
        let renewed = now - Duration::minutes(1);
        b_billing
            .save_from("codex", UUID, "Pro 5x", Some(end), renewed, "browser")
            .unwrap();
        assert!(b_billing.peek_for("codex", UUID, now).is_some());
    }

    #[test]
    fn imported_dates_require_exact_uuid_and_fresh_matching_local_plan() {
        let now = Utc::now();
        let a = tempfile::tempdir().unwrap();
        let (a_store, a_billing) = stores(a.path());
        let observed = now - Duration::minutes(5);
        a_billing
            .save_from(
                "codex",
                UUID,
                "Pro 5x",
                Some(now + Duration::days(5)),
                observed,
                "browser",
            )
            .unwrap();
        let incoming = a_store.export(&local(now), &a_billing, now).unwrap();
        for condition in ["uuid", "plan", "stale", "future", "missing"] {
            let b = tempfile::tempdir().unwrap();
            let (store, billing) = stores(b.path());
            let mut current = local(now);
            match condition {
                "uuid" => current.accounts[0].metadata.account_uuid = OTHER_UUID.into(),
                "plan" => current.accounts[0].plan = Some("Plus".into()),
                "stale" => current.accounts[0].plan_checked_at = Some(now - Duration::hours(1)),
                "future" => current.accounts[0].plan_checked_at = Some(now + Duration::minutes(1)),
                _ => current.accounts.clear(),
            }
            let result = store.import(&incoming, &current, &billing, now).unwrap();
            assert_eq!(result.report.billing_applied, 0, "{condition}");
            assert_eq!(result.report.billing_pending, 1, "{condition}");
            assert!(
                billing.peek_for("codex", UUID, now).is_none(),
                "{condition}"
            );
        }
        let b = tempfile::tempdir().unwrap();
        let (store, billing) = stores(b.path());
        store.import(&incoming, &local(now), &billing, now).unwrap();
        assert!(
            matches!(billing.peek_for("codex", UUID, now), Some(uc_core::PlanTerm::Stated { checked_at: Some(at), .. }) if at == observed)
        );
    }

    #[test]
    fn billing_updates_track_only_changed_local_accounts_not_preference_imports() {
        let now = Utc::now();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let (a_store, a_billing) = stores(a.path());
        let (b_store, b_billing) = stores(b.path());
        let mut a_local = local(now);
        let mut b_local = local(now);
        let expected_id = b_local.accounts[0].metadata.id.clone();
        let mut unaffected = b_local.accounts[0].clone();
        unaffected.metadata.id = format!("codex@{}", "b".repeat(64));
        unaffected.metadata.account_uuid = OTHER_UUID.into();
        b_local.accounts.push(unaffected);
        a_billing
            .save_from(
                "codex",
                UUID,
                "Pro 5x",
                Some(now + Duration::days(5)),
                now - Duration::minutes(5),
                "browser",
            )
            .unwrap();
        let initial = a_store.export(&a_local, &a_billing, now).unwrap();
        let result = b_store.import(&initial, &b_local, &b_billing, now).unwrap();
        use_result(&mut b_local, &result);
        assert_eq!(
            b_store.billing_updates_since(0).unwrap(),
            vec![expected_id.clone()]
        );
        let initial_revision = result.report.revision;
        a_local.settings["language"] = json!("vi");
        let preferences = a_store.export(&a_local, &a_billing, now).unwrap();
        let result = b_store
            .import(&preferences, &b_local, &b_billing, now)
            .unwrap();
        use_result(&mut b_local, &result);
        assert!(result.report.settings_changed);
        assert!(
            b_store
                .billing_updates_since(initial_revision)
                .unwrap()
                .is_empty()
        );
        let preference_revision = result.report.revision;
        a_billing
            .invalidate_for_at("codex", UUID, now - Duration::minutes(1))
            .unwrap();
        a_billing
            .invalidate_for_at("codex", &Uuid::new_v4().to_string(), now)
            .unwrap();
        let cancellation = a_store.export(&a_local, &a_billing, now).unwrap();
        let result = b_store
            .import(&cancellation, &b_local, &b_billing, now)
            .unwrap();
        use_result(&mut b_local, &result);
        assert_eq!(result.report.billing_applied, 1);
        assert_eq!(result.report.billing_pending, 1);
        assert_eq!(
            b_store.billing_updates_since(preference_revision).unwrap(),
            vec![expected_id]
        );
        assert!(
            b_store
                .billing_updates_since(result.report.revision)
                .unwrap()
                .is_empty()
        );
        let replay = b_store
            .import(&cancellation, &b_local, &b_billing, now)
            .unwrap();
        assert!(!replay.report.changed);
        assert_eq!(replay.report.revision, result.report.revision);
    }

    #[test]
    fn preference_and_roster_tombstones_survive_offline_replays() {
        let now = Utc::now();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let (a_store, a_billing) = stores(a.path());
        let (b_store, b_billing) = stores(b.path());
        let mut a_local = local(now);
        let mut b_local = local(now);
        let old = a_store.export(&a_local, &a_billing, now).unwrap();
        let result = b_store.import(&old, &b_local, &b_billing, now).unwrap();
        use_result(&mut b_local, &result);
        a_local.settings.as_object_mut().unwrap().remove("language");
        let id = a_local.accounts[0].metadata.id.clone();
        a_store.remove_account(&id).unwrap();
        let removed = a_store.export(&a_local, &a_billing, now).unwrap();
        assert!(removed.accounts[&id].account.is_none());
        let result = b_store.import(&removed, &b_local, &b_billing, now).unwrap();
        use_result(&mut b_local, &result);
        assert!(b_local.settings.get("language").is_none());
        let result = b_store.import(&old, &b_local, &b_billing, now).unwrap();
        assert!(result.settings.get("language").is_none());
        assert!(
            b_store.export(&b_local, &b_billing, now).unwrap().accounts[&id]
                .account
                .is_none()
        );
    }

    #[test]
    fn unknown_fields_sources_sizes_and_machine_paths_are_rejected() {
        let now = Utc::now();
        let dir = tempfile::tempdir().unwrap();
        let (store, billing) = stores(dir.path());
        let envelope = store.export(&local(now), &billing, now).unwrap();
        let mut value = serde_json::to_value(&envelope).unwrap();
        value["tokens"] = json!({"refresh_token":"must-not-import"});
        assert!(Envelope::from_slice(&serde_json::to_vec(&value).unwrap()).is_err());
        assert!(Envelope::from_slice(&vec![b' '; MAX_BYTES + 1]).is_err());
        let mut bad = envelope.clone();
        bad.fields.insert(
            "settings/browserProfile".into(),
            Field {
                revision: Revision {
                    counter: 1,
                    device_id: Uuid::new_v4(),
                },
                value: Some(json!("C:/Profile")),
            },
        );
        assert!(bad.validate(now).is_err());
        let mut bad = envelope.clone();
        bad.fields.values_mut().next().unwrap().revision.device_id = Uuid::nil();
        assert!(bad.validate(now).is_err());
        let bytes = envelope.to_bytes().unwrap();
        let text = String::from_utf8(bytes).unwrap();
        for excluded in ["knownProviders", "refresh_token", "browserProfile"] {
            assert!(!text.contains(excluded));
        }
    }

    #[test]
    fn stale_ui_delta_preserves_imported_fields_and_nested_changes() {
        let dir = tempfile::tempdir().unwrap();
        let before = json!({"theme":"light","language":"en","notifications":{"almostOut":true,"willRunOut":false},"knownProviders":["local"]});
        let imported = json!({"theme":"light","language":"vi","notifications":{"almostOut":false,"willRunOut":false},"knownProviders":["local"]});
        let next = json!({"theme":"dark","language":"en","notifications":{"almostOut":true,"willRunOut":true},"knownProviders":["local"]});
        uc_core::paths::write_atomic(
            &dir.path().join("settings.json"),
            &serde_json::to_vec(&imported).unwrap(),
        )
        .unwrap();
        save_document_changes_at(dir.path(), "settings", Some(&before), &next).unwrap();
        let saved = read_document(&dir.path().join("settings.json")).unwrap();
        assert_eq!(saved["theme"], "dark");
        assert_eq!(saved["language"], "vi");
        assert_eq!(
            saved["notifications"],
            json!({"almostOut":false,"willRunOut":true})
        );
        assert_eq!(saved["knownProviders"], json!(["local"]));
    }

    #[test]
    fn settings_schema_preserves_current_choices_and_normalizes_known_legacy_island_fields() {
        let now = Utc::now();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let (a_store, a_billing) = stores(a.path());
        let (b_store, b_billing) = stores(b.path());
        let mut a_local = local(now);
        a_local.settings = json!({"settingsRevision":1,"island":{"showPlan":false}});
        let incoming = a_store.export(&a_local, &a_billing, now).unwrap();
        let mut b_local = local(now);
        b_local.settings = json!({});
        let imported = b_store
            .import(&incoming, &b_local, &b_billing, now)
            .unwrap();
        assert_eq!(imported.settings["settingsRevision"], 1);
        assert_eq!(imported.settings["island"]["showPlan"], false);
        let legacy = json!({"island":{"showPlan":false,"layout":"separate","view":"resets","sections":{"quota":true,"upcoming":true}}});
        let fields = portable_fields(&legacy, &json!({})).unwrap();
        assert_eq!(fields["settings/settingsRevision"], 1);
        assert_eq!(fields["settings/island/showPlan"], true);
        assert_eq!(
            fields["settings/island/tabs"],
            json!(["resets", "quota", "upcoming"])
        );
        assert!(
            !fields
                .keys()
                .any(|key| key.contains("sections") || key.ends_with("/view"))
        );
        let legacy = json!({"island":{"layout":"combined","sections":{}}});
        assert_eq!(
            portable_fields(&legacy, &json!({})).unwrap()["settings/island/tabs"],
            json!(["quota"])
        );
        let mut invalid = incoming;
        invalid
            .fields
            .get_mut("settings/settingsRevision")
            .unwrap()
            .value = Some(json!(u64::MAX));
        assert!(invalid.validate(now).is_err());
    }

    #[test]
    fn provider_choices_stay_separate_and_foreign_choices_are_not_deleted() {
        let now = Utc::now();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let (a_store, a_billing) = stores(a.path());
        let (b_store, b_billing) = stores(b.path());
        let mut a_local = local(now);
        let mut b_local = local(now);
        a_local.settings["knownProviders"] = json!(["claude-local"]);
        a_local.settings["enabledProviders"] = json!([]);
        b_local.settings["knownProviders"] = json!(["codex-local"]);
        b_local.settings["enabledProviders"] = json!(["codex-local"]);
        let a_out = a_store.export(&a_local, &a_billing, now).unwrap();
        let result = b_store.import(&a_out, &b_local, &b_billing, now).unwrap();
        use_result(&mut b_local, &result);
        assert_eq!(b_local.settings["enabledProviders"], json!(["codex-local"]));
        let b_out = b_store.export(&b_local, &b_billing, now).unwrap();
        assert_eq!(
            b_out.fields["settings/enabledProviders/claude-local"].value,
            Some(json!(false))
        );
        assert_eq!(
            b_out.fields["settings/enabledProviders/codex-local"].value,
            Some(json!(true))
        );
        b_local.settings["knownProviders"] = json!(["codex-local", "claude-local"]);
        b_local.settings["enabledProviders"] = json!(["codex-local", "claude-local"]);
        let result = b_store.import(&a_out, &b_local, &b_billing, now).unwrap();
        assert_eq!(result.settings["enabledProviders"], json!(["codex-local"]));
        use_result(&mut b_local, &result);
        b_local.settings["knownProviders"] = json!(["codex-local"]);
        let b_out = b_store.export(&b_local, &b_billing, now).unwrap();
        assert_eq!(
            b_out.fields["settings/enabledProviders/claude-local"].value,
            Some(json!(false))
        );
    }

    #[test]
    fn removal_before_first_export_needs_an_explicit_restore() {
        let now = Utc::now();
        let directory = tempfile::tempdir().unwrap();
        let (store, billing) = stores(directory.path());
        let local = local(now);
        let account = local.accounts[0].metadata.clone();
        store.remove_account(&account.id).unwrap();
        assert!(
            store.export(&local, &billing, now).unwrap().accounts[&account.id]
                .account
                .is_none()
        );
        store.restore_account(account.clone()).unwrap();
        assert_eq!(
            store.export(&local, &billing, now).unwrap().accounts[&account.id].account,
            Some(account)
        );
    }

    #[test]
    fn fresh_paid_account_without_local_billing_accepts_existing_confirmed_evidence() {
        let now = Utc::now();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let (a_store, a_billing) = stores(a.path());
        let (b_store, b_billing) = stores(b.path());
        let observed = now - Duration::days(2);
        a_billing
            .save_from(
                "codex",
                UUID,
                "Pro 5x",
                Some(now + Duration::days(5)),
                observed,
                "browser",
            )
            .unwrap();
        assert!(
            b_billing
                .get_for("codex", UUID, Some("Pro 5x"), now)
                .is_none()
        );
        assert!(b_billing.sync_evidence(now).unwrap().is_empty());
        let incoming = a_store.export(&local(now), &a_billing, now).unwrap();
        b_store
            .import(&incoming, &local(now), &b_billing, now)
            .unwrap();
        assert!(
            matches!(b_billing.peek_for("codex", UUID, now), Some(uc_core::PlanTerm::Stated { checked_at: Some(at), .. }) if at == observed)
        );
    }

    #[test]
    fn foreign_layout_entries_survive_import_and_known_removals_stay_removed() {
        let now = Utc::now();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let (a_store, a_billing) = stores(a.path());
        let (b_store, b_billing) = stores(b.path());
        let mut a_local = local(now);
        let mut b_local = local(now);
        b_local.layout = json!({});
        a_local.layout = json!({"version":1,"providerOrder":["foreign","local"],"placed":["foreign.weekly","local.weekly"],"pinned":["foreign.weekly"],"metricOrder":{"foreign":["foreign.weekly"]}});
        let original = a_store.export(&a_local, &a_billing, now).unwrap();
        b_store.export(&b_local, &b_billing, now).unwrap();
        a_local.layout["placed"] = json!(["foreign.weekly"]);
        let changed = a_store.export(&a_local, &a_billing, now).unwrap();
        let result = b_store.import(&changed, &b_local, &b_billing, now).unwrap();
        use_result(&mut b_local, &result);
        assert_eq!(b_local.layout["version"], 1);
        assert_eq!(b_local.layout["placed"], json!(["foreign.weekly"]));
        assert_eq!(
            b_local.layout["metricOrder"]["foreign"],
            json!(["foreign.weekly"])
        );
        let outgoing = b_store.export(&b_local, &b_billing, now).unwrap();
        assert_eq!(
            outgoing.fields["layout/placed"].value,
            Some(json!(["foreign.weekly"]))
        );
        let replay = b_store
            .import(&original, &b_local, &b_billing, now)
            .unwrap();
        assert_eq!(replay.layout["placed"], json!(["foreign.weekly"]));
    }
}
