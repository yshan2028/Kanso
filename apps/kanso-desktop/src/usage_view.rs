//! 用量页面的连续时间轴、范围汇总和数字格式。

use crate::{UsageDim, UsageRange};
use chrono::{Datelike, Days, NaiveDate};
use kanso_usage::{DailyBucket, Family, SourceKind, UsageSnapshot};
use std::cmp::Reverse;
use std::collections::BTreeMap;

const DAY_FORMAT: &str = "%Y-%m-%d";

/// 连续时间轴上的一根堆叠柱。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SeriesBar {
    /// 本地日期或小时键。
    pub key: String,
    /// 柱的显示标签。
    pub label: String,
    /// 未命中缓存的输入词元与缓存写入词元之和。
    pub in_u: u64,
    /// 缓存读取词元数。
    pub cache_read: u64,
    /// 输出词元数。
    pub output: u64,
    /// 三段词元之和。
    pub total: u64,
    /// 请求次数。
    pub calls: u64,
}

impl SeriesBar {
    fn add(&mut self, input: u64, cache_write: u64, cache_read: u64, output: u64, calls: u64) {
        self.in_u = self.in_u.saturating_add(input).saturating_add(cache_write);
        self.cache_read = self.cache_read.saturating_add(cache_read);
        self.output = self.output.saturating_add(output);
        self.total = self
            .in_u
            .saturating_add(self.cache_read)
            .saturating_add(self.output);
        self.calls = self.calls.saturating_add(calls);
    }
}

/// 连续时间轴及其刻度和摘要。
#[derive(Debug, PartialEq, Eq)]
pub struct SeriesSummary {
    /// 按时间升序排列的连续柱列。
    pub bars: Vec<SeriesBar>,
    /// 是否以小时为单位。
    pub is_hourly: bool,
    /// 纵轴上限。
    pub y_max: u64,
    /// 峰值所在日期或小时。
    pub peak_label: String,
    /// 峰值词元数。
    pub peak_tokens: u64,
    /// 有请求或词元记录的槽数。
    pub active_count: usize,
    /// 活跃槽的平均词元数，四舍五入到整数。
    pub avg_active: u64,
    /// 需要显示横轴标签的柱索引。
    pub tick_indices: Vec<usize>,
}

/// 给出词元数对应的纵轴整刻度上限，空轴取 100。
pub fn nice_max(tokens: u64) -> u64 {
    if tokens == 0 {
        return 100;
    }
    let power = 10_u64.pow(tokens.ilog10());
    for multiplier in [1, 2, 5] {
        let ceiling = power.saturating_mul(multiplier);
        if tokens <= ceiling {
            return ceiling;
        }
    }
    power.saturating_mul(10)
}

/// 构造所选本地日历范围的时间轴，填齐没有记录的时间槽。
pub fn build_series(
    snapshot: &UsageSnapshot,
    range: UsageRange,
    today: NaiveDate,
) -> SeriesSummary {
    let is_hourly = range == UsageRange::Today;
    let bars = if is_hourly {
        let mut bars: Vec<_> = (0..24)
            .map(|hour| {
                let key = format!("{hour:02}:00");
                SeriesBar {
                    label: key.clone(),
                    key,
                    ..SeriesBar::default()
                }
            })
            .collect();
        for slot in &snapshot.hourly_today {
            if let Some(bar) = bars.get_mut(usize::from(slot.hour)) {
                bar.add(
                    slot.input_uncached,
                    slot.cache_write,
                    slot.cache_read,
                    slot.output,
                    slot.calls,
                );
            }
        }
        bars
    } else {
        let start = range_start(range, today).or_else(|| {
            snapshot
                .daily
                .keys()
                .filter_map(|key| NaiveDate::parse_from_str(key, DAY_FORMAT).ok())
                .filter(|day| *day <= today)
                .min()
        });
        std::iter::successors(Some(start.unwrap_or(today)), NaiveDate::succ_opt)
            .take_while(|day| *day <= today)
            .map(|day| {
                let key = day.format(DAY_FORMAT).to_string();
                let mut bar = SeriesBar {
                    label: format!("{}月{}日", day.month(), day.day()),
                    key,
                    ..SeriesBar::default()
                };
                for row in snapshot.daily.get(&bar.key).into_iter().flatten() {
                    bar.add(
                        row.input_uncached,
                        row.cache_write,
                        row.cache_read,
                        row.output,
                        row.calls,
                    );
                }
                bar
            })
            .collect()
    };

    let peak = bars.iter().rev().max_by_key(|bar| bar.total);
    let peak_tokens = peak.map_or(0, |bar| bar.total);
    let peak_label = peak.map_or_else(|| "—".to_string(), |bar| bar.label.clone());
    let active_count = bars
        .iter()
        .filter(|bar| bar.calls > 0 || bar.total > 0)
        .count();
    let sum = bars.iter().map(|bar| u128::from(bar.total)).sum::<u128>();
    let active = active_count as u128;
    let avg_active = (sum + active / 2).checked_div(active).unwrap_or_default() as u64;
    let tick_indices = if is_hourly {
        vec![0, 4, 8, 12, 16, 20, 23]
    } else if bars.len() <= 7 {
        (0..bars.len()).collect()
    } else {
        (0..7).map(|i| (i * (bars.len() - 1) + 3) / 6).collect()
    };

    SeriesSummary {
        bars,
        is_hourly,
        y_max: nice_max(peak_tokens),
        peak_label,
        peak_tokens,
        active_count,
        avg_active,
        tick_indices,
    }
}

/// 一组会话日志的总量。
#[derive(Debug, Default, PartialEq)]
pub struct UsageTotals {
    /// 估算费用，单位为美元。
    pub est_usd: f64,
    /// 输入词元数，含缓存读取与缓存写入。
    pub input: u64,
    /// 读取的缓存词元数。
    pub cache_read: u64,
    /// 输出词元数。
    pub output: u64,
    /// 总词元数。
    pub tokens: u64,
    /// 请求数。
    pub calls: u64,
    /// 失败请求数。
    pub errors: u64,
}

impl UsageTotals {
    fn add(&mut self, row: &DailyBucket) {
        self.est_usd += row.est_usd;
        let input = row
            .input_uncached
            .saturating_add(row.cache_write)
            .saturating_add(row.cache_read);
        self.input = self.input.saturating_add(input);
        self.cache_read = self.cache_read.saturating_add(row.cache_read);
        self.output = self.output.saturating_add(row.output);
        self.tokens = self.input.saturating_add(self.output);
        self.calls = self.calls.saturating_add(row.calls);
        self.errors = self.errors.saturating_add(row.errors);
    }

    /// 缓存读取占总词元的百分比，沿用原型统计口径。
    pub fn cache_hit_pct(&self) -> f64 {
        percent(self.cache_read, self.tokens)
    }
}

/// 一行按所选维度分组的用量。
#[derive(Debug, PartialEq)]
pub struct BreakdownRow {
    /// 分组显示名。
    pub label: String,
    /// 此分组的汇总数据。
    pub totals: UsageTotals,
}

/// 所选时间范围的汇总和明细。
#[derive(Debug, PartialEq)]
pub struct RangeSummary {
    /// 范围内的总量。
    pub totals: UsageTotals,
    /// 按词元量降序排列的明细。
    pub breakdown: Vec<BreakdownRow>,
}

/// 汇总所选本地日历范围内的每日统计及指定维度的分组。
pub fn summarize_range(
    snapshot: &UsageSnapshot,
    range: UsageRange,
    today: NaiveDate,
    dim: UsageDim,
) -> RangeSummary {
    let start = range_start(range, today);
    let mut totals = UsageTotals::default();
    let mut groups = BTreeMap::<&str, UsageTotals>::new();

    for (key, rows) in &snapshot.daily {
        let Ok(day) = NaiveDate::parse_from_str(key, DAY_FORMAT) else {
            continue;
        };
        if !in_range(day, start, today) {
            continue;
        }
        for row in rows {
            let label = match dim {
                UsageDim::Provider => family_label(&row.family),
                UsageDim::Model => model_label(row.model.as_deref()),
                UsageDim::Agent => source_label(&row.source_kind),
            };
            totals.add(row);
            groups.entry(label).or_default().add(row);
        }
    }

    let mut breakdown: Vec<_> = groups
        .into_iter()
        .map(|(label, totals)| BreakdownRow {
            label: label.to_string(),
            totals,
        })
        .collect();
    breakdown.sort_by_key(|row| Reverse(row.totals.tokens));
    RangeSummary { totals, breakdown }
}

/// 计算统计和请求视图共用的本地日期下限，含首日。
pub(crate) fn range_start(range: UsageRange, today: NaiveDate) -> Option<NaiveDate> {
    let days_back = match range {
        UsageRange::Today => 0,
        UsageRange::D7 => 6,
        UsageRange::D30 => 29,
        UsageRange::All => return None,
    };
    Some(
        today
            .checked_sub_days(Days::new(days_back))
            .unwrap_or(NaiveDate::MIN),
    )
}

/// 判断日期是否落在含首尾的本地日历区间内。
pub(crate) fn in_range(day: NaiveDate, start: Option<NaiveDate>, today: NaiveDate) -> bool {
    day <= today && start.is_none_or(|start| day >= start)
}

/// 返回会话日志能够识别的供应商家族名称。
pub(crate) fn family_label(family: &Family) -> &'static str {
    match family {
        Family::Anthropic => "Anthropic · Claude Code",
        Family::OpenAi => "OpenAI · Codex",
        Family::Google => "Google · Gemini",
    }
}

/// 返回会话来源的显示名。
pub(crate) fn source_label(source: &SourceKind) -> &'static str {
    match source {
        SourceKind::ClaudeCode => "Claude Code",
        SourceKind::CodexCli => "Codex CLI",
        SourceKind::CodexDesktop => "ChatGPT 桌面端（Codex）",
        SourceKind::CodexExec => "Codex 无头执行",
    }
}

/// 显示模型名称及其缺失状态。
pub(crate) fn model_label(model: Option<&str>) -> &str {
    match model {
        Some(model) => model,
        None => "未知模型",
    }
}

/// 返回百分比，分母为零时结果为零。
pub(crate) fn percent(part: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        part as f64 / total as f64 * 100.0
    }
}

/// 将词元或请求数缩写为 B、M、k。
pub fn format_short_num(value: u64) -> String {
    match value {
        1_000_000_000.. => format!("{:.2}B", value as f64 / 1_000_000_000.0),
        1_000_000.. => format!("{:.1}M", value as f64 / 1_000_000.0),
        1_000.. => format!("{:.1}k", value as f64 / 1_000.0),
        _ => value.to_string(),
    }
}

/// 显示美元费用，保留小额正费用的四位小数。
pub fn format_usd(value: f64) -> String {
    let precision = if value > 0.0 && value < 0.01 { 4 } else { 2 };
    let digits = format!("{:.precision$}", value.abs());
    let mut display = if value.is_sign_negative() {
        String::from("$-")
    } else {
        String::from("$")
    };
    if let Some((whole, fraction)) = digits.split_once('.') {
        for (index, digit) in whole.chars().enumerate() {
            if index > 0 && (whole.len() - index).is_multiple_of(3) {
                display.push(',');
            }
            display.push(digit);
        }
        display.push('.');
        display.push_str(fraction);
    } else {
        display.push_str(&digits);
    }
    display
}

#[cfg(test)]
mod tests {
    use super::*;
    use kanso_usage::{Coverage, HourlySlot};

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 27).unwrap()
    }

    fn bucket(family: Family, source_kind: SourceKind, model: Option<&str>) -> DailyBucket {
        DailyBucket {
            source_kind,
            family,
            model: model.map(str::to_string),
            calls: 2,
            errors: 1,
            input_uncached: 80,
            cache_read: 50,
            cache_write: 20,
            output: 50,
            reasoning: 10,
            est_usd: 0.25,
        }
    }

    fn snapshot() -> UsageSnapshot {
        let claude = bucket(Family::Anthropic, SourceKind::ClaudeCode, Some("claude"));
        let codex = bucket(Family::OpenAi, SourceKind::CodexCli, Some("gpt"));
        UsageSnapshot {
            requests: Vec::new(),
            total_requests: 6,
            total_errors: 3,
            daily: BTreeMap::from([
                ("2026-10-18".to_string(), vec![claude.clone()]),
                ("2026-11-05".to_string(), vec![claude]),
                ("2026-11-27".to_string(), vec![codex]),
            ]),
            hourly_today: vec![HourlySlot {
                hour: 14,
                calls: 2,
                input_uncached: 80,
                cache_read: 50,
                cache_write: 20,
                output: 50,
            }],
            coverage: Coverage {
                lookback_days: 60,
                first_day: "2026-10-18".to_string(),
                last_day: "2026-11-27".to_string(),
                active_days: 3,
            },
            quota: Vec::new(),
        }
    }

    #[test]
    fn calendar_is_continuous_through_today_for_every_daily_range() {
        let snapshot = snapshot();
        let month = build_series(&snapshot, UsageRange::D30, today());
        assert_eq!(month.bars.len(), 30);
        assert_eq!(month.bars[0].key, "2026-10-29");
        assert_eq!(month.bars[7].total, 200);
        assert_eq!(month.bars[8].key, "2026-11-06");
        assert_eq!(month.bars[8].total, 0);
        assert_eq!(month.bars[29].key, "2026-11-27");
        assert_eq!(month.tick_indices, [0, 5, 10, 15, 19, 24, 29]);

        let week = build_series(&snapshot, UsageRange::D7, today());
        assert_eq!(week.bars.len(), 7);
        assert_eq!(week.bars[0].key, "2026-11-21");
        assert_eq!(week.tick_indices, [0, 1, 2, 3, 4, 5, 6]);

        let all = build_series(&snapshot, UsageRange::All, today());
        assert_eq!(all.bars.len(), 41);
        assert_eq!(all.bars[0].key, "2026-10-18");
        assert_eq!(all.bars[40].key, "2026-11-27");
        assert_eq!(all.active_count, 3);
    }

    #[test]
    fn today_has_24_ordered_hours_and_the_prototype_ticks() {
        let series = build_series(&snapshot(), UsageRange::Today, today());
        assert!(series.is_hourly);
        assert_eq!(series.bars.len(), 24);
        assert_eq!(series.bars[0].label, "00:00");
        assert_eq!(series.bars[13].calls, 0);
        assert_eq!(series.bars[14].in_u, 100);
        assert_eq!(series.bars[14].cache_read, 50);
        assert_eq!(series.bars[14].output, 50);
        assert_eq!(series.bars[14].calls, 2);
        assert_eq!(series.bars[23].label, "23:00");
        assert_eq!(series.tick_indices, [0, 4, 8, 12, 16, 20, 23]);
        assert_eq!(series.peak_label, "14:00");
        assert_eq!(series.peak_tokens, 200);
    }

    #[test]
    fn axis_uses_one_two_five_and_powers_of_ten() {
        for (value, expected) in [
            (0, 100),
            (1, 1),
            (2, 2),
            (3, 5),
            (5, 5),
            (6, 10),
            (11, 20),
            (2_001, 5_000),
            (51_001, 100_000),
            (u64::MAX, u64::MAX),
        ] {
            assert_eq!(nice_max(value), expected, "输入 {value}");
        }
    }

    #[test]
    fn peak_keeps_first_tie_and_average_uses_only_active_slots() {
        let mut snapshot = snapshot();
        let series = build_series(&snapshot, UsageRange::D30, today());
        assert_eq!(series.peak_label, "11月5日");
        assert_eq!(series.active_count, 2);
        assert_eq!(series.avg_active, 200);

        let mut failed = bucket(Family::OpenAi, SourceKind::CodexExec, None);
        failed.input_uncached = 0;
        failed.output = 0;
        failed.cache_read = 0;
        failed.cache_write = 0;
        snapshot.daily.insert("2026-11-26".into(), vec![failed]);
        let series = build_series(&snapshot, UsageRange::D30, today());
        assert_eq!(series.active_count, 3);
        assert_eq!(series.avg_active, 133);

        snapshot.daily.clear();
        let empty = build_series(&snapshot, UsageRange::All, today());
        assert_eq!(empty.bars.len(), 1);
        assert_eq!(empty.avg_active, 0);
        assert_eq!(empty.y_max, 100);
    }

    #[test]
    fn totals_and_breakdown_follow_the_same_calendar_range() {
        let mut snapshot = snapshot();
        let other = bucket(Family::OpenAi, SourceKind::CodexDesktop, None);
        snapshot.daily.get_mut("2026-11-27").unwrap().push(other);
        snapshot.daily.insert(
            "2026-11-28".into(),
            vec![bucket(Family::OpenAi, SourceKind::CodexCli, Some("未来"))],
        );

        let range = summarize_range(&snapshot, UsageRange::D30, today(), UsageDim::Provider);
        assert_eq!(range.totals.est_usd, 0.75);
        assert_eq!(range.totals.calls, 6);
        assert_eq!(range.totals.errors, 3);
        assert_eq!(range.totals.tokens, 600);
        assert_eq!(range.totals.input, 450);
        assert_eq!(range.totals.output, 150);
        assert_eq!(range.totals.cache_hit_pct(), 25.0);
        assert_eq!(range.breakdown.len(), 2);
        assert_eq!(range.breakdown[0].label, "OpenAI · Codex");
        assert_eq!(range.breakdown[0].totals.tokens, 400);
        assert_eq!(range.breakdown[1].label, "Anthropic · Claude Code");

        let models = summarize_range(&snapshot, UsageRange::Today, today(), UsageDim::Model);
        assert_eq!(models.totals.calls, 4);
        assert_eq!(models.breakdown.len(), 2);
        assert!(models.breakdown.iter().any(|row| row.label == "未知模型"));

        let agents = summarize_range(&snapshot, UsageRange::D7, today(), UsageDim::Agent);
        assert_eq!(agents.breakdown.len(), 2);
        assert_eq!(agents.breakdown[0].label, "ChatGPT 桌面端（Codex）");
        assert_eq!(agents.breakdown[1].label, "Codex CLI");
    }

    #[test]
    fn number_formats_match_the_prototype_precision() {
        assert_eq!(format_short_num(999), "999");
        assert_eq!(format_short_num(1_234), "1.2k");
        assert_eq!(format_short_num(2_340_000), "2.3M");
        assert_eq!(format_short_num(3_450_000_000), "3.45B");
        assert_eq!(format_usd(0.0), "$0.00");
        assert_eq!(format_usd(0.0042), "$0.0042");
        assert_eq!(format_usd(12_345.25), "$12,345.25");
    }
}
