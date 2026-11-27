use crate::model::{Family, ProviderEntry, ProviderSource, ToolVersion, UsageSnapshot};

/// 按模型家族汇总会话用量，并补充已安装的工具及版本。
pub fn derive_providers(snapshot: &UsageSnapshot, tools: &[ToolVersion]) -> Vec<ProviderEntry> {
    let mut providers = Vec::new();
    for (day, buckets) in &snapshot.daily {
        for bucket in buckets {
            let entry = provider_for(&mut providers, &bucket.family, ProviderSource::SessionLog);
            if let Some(model) = &bucket.model {
                entry.models.push(model.clone());
            }
            entry.calls = entry.calls.saturating_add(bucket.calls);
            entry.errors = entry.errors.saturating_add(bucket.errors);
            entry.tokens = entry
                .tokens
                .saturating_add(bucket.input_uncached)
                .saturating_add(bucket.cache_write)
                .saturating_add(bucket.cache_read)
                .saturating_add(bucket.output);
            entry.last_day = Some(day.clone());
        }
    }

    for tool in tools.iter().filter(|tool| tool.installed) {
        let entry = provider_for(&mut providers, &tool.family, ProviderSource::CliProbe);
        if entry.tool == tool.tool && entry.version.is_none() {
            entry.version.clone_from(&tool.version);
        }
    }

    for entry in &mut providers {
        entry.models.sort_unstable();
        entry.models.dedup();
    }
    providers
}

fn provider_for<'a>(
    providers: &'a mut Vec<ProviderEntry>,
    family: &Family,
    source: ProviderSource,
) -> &'a mut ProviderEntry {
    if let Some(index) = providers.iter().position(|entry| &entry.family == family) {
        return &mut providers[index];
    }

    let (id, name, tool) = match family {
        Family::Anthropic => ("local-anthropic", "Claude Code", "claude"),
        Family::OpenAi => ("local-openai", "Codex", "codex"),
        Family::Google => ("local-google", "Gemini CLI", "gemini"),
    };
    let index = providers.len();
    providers.push(ProviderEntry {
        id: id.to_owned(),
        name: name.to_owned(),
        family: family.clone(),
        source,
        tool: tool.to_owned(),
        version: None,
        models: Vec::new(),
        calls: 0,
        errors: 0,
        tokens: 0,
        last_day: None,
    });
    &mut providers[index]
}
