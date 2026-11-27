use crate::Workbench;
use crate::settings_view::{ALL_POLICIES, build_candidates, policy_key};
use gpui_kit::base::{Selectable as _, TestTextExt as _};
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme, Sizable as _};
use gpui_kit::{
    AnyElement, App, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div,
    px,
};
use kanso_core::routing::{Role, RolePolicy, RouteOutcome, RoutingInput, SkipReason, plan_route};

use super::settings::{page_notice, persist_settings, section_card};

/// 渲染路由策略、任务路由步骤和各角色的候选顺序预览。
pub fn render_routing(
    this: &Workbench,
    _window: &mut Window,
    cx: &mut Context<Workbench>,
) -> AnyElement {
    let policy = this.settings.routing_policy;
    let policies = div()
        .flex()
        .flex_wrap()
        .gap_2()
        .children(ALL_POLICIES.into_iter().map(|option| {
            Button::new(SharedString::from(format!(
                "routing-policy-{}",
                policy_key(option)
            )))
            .small()
            .outline()
            .rounded(px(999.0))
            .selected(option == policy)
            .label(option.label_zh())
            .on_click(cx.listener(move |this, _, _, cx| {
                persist_settings(this, |settings| {
                    settings.routing_policy = option;
                });
                cx.notify();
            }))
        }));
    div()
        .id("page-routing")
        .size_full()
        .overflow_y_scroll()
        .text_color(cx.theme().foreground)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_4()
                .p_6()
                .child(
                    div()
                        .text_size(px(22.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("路由"),
                )
                .children(page_notice(this, cx))
                .child(section_card("路由策略", cx).child(policies))
                .child(pipeline(policy, cx))
                .child(role_candidates(this, policy, cx)),
        )
        .test_support()
        .into_any_element()
}

fn pipeline(policy: RolePolicy, cx: &App) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    let steps = [
        ("确定任务角色", "识别主控、执行者和审核者"),
        ("收集供应商", "识别可用于任务的本地命令行工具"),
        ("筛选候选", "检查启用状态、角色能力和模型家族"),
        ("检查运行指标", "查看近期失败率与剩余额度"),
        ("按策略排序", policy.label_zh()),
        ("分派任务", "直接调度本地命令行工具执行任务"),
    ];

    section_card("任务路由决策", cx).children(steps.into_iter().enumerate().map(
        |(index, (title, detail))| {
            let number = index + 1;
            div()
                .id(("routing-step", number))
                .flex()
                .gap_3()
                .child(
                    div()
                        .w(px(20.0))
                        .flex_shrink_0()
                        .text_color(muted)
                        .child(number.to_string()),
                )
                .child(div().w(px(105.0)).flex_shrink_0().child(title))
                .child(
                    div()
                        .min_w_0()
                        .text_color(muted)
                        .child(detail.test_text(detail)),
                )
                .test_support()
        },
    ))
}

fn role_candidates(this: &Workbench, policy: RolePolicy, cx: &App) -> impl IntoElement {
    let providers = &this.state.providers;
    let candidates = build_candidates(providers, &this.settings, this.state.usage.as_ref());
    let plan = |role, worker_family| {
        plan_route(
            &candidates,
            &RoutingInput {
                role,
                policy,
                worker_family,
                sticky_id: None,
                rr_cursor: 0,
            },
        )
    };
    let worker = plan(Role::Worker, None);
    let worker_family = match &worker {
        RouteOutcome::Assigned { ordered, .. } => ordered
            .first()
            .and_then(|id| candidates.iter().find(|candidate| &candidate.id == id))
            .map(|candidate| candidate.family.clone()),
        RouteOutcome::Escalate { .. } => None,
    };
    let lead = plan(Role::Lead, None);
    let reviewer = plan(Role::Reviewer, worker_family);
    let name = |id: String| {
        providers
            .iter()
            .find(|provider| provider.id == id)
            .map_or(id, |provider| provider.name.clone())
    };
    let theme = cx.theme();

    section_card("各角色候选顺序", cx)
        .child(
            div()
                .text_color(theme.muted_foreground)
                .child("审核者以首位执行候选的模型家族为参照。"),
        )
        .child(
            div()
                .flex()
                .gap_3()
                .text_color(theme.muted_foreground)
                .child(div().w(px(66.0)).flex_shrink_0().child("角色"))
                .child(div().flex_1().child("通过筛选的候选顺序"))
                .child(div().flex_1().child("跳过原因")),
        )
        .children(
            [("主控", lead), ("执行者", worker), ("审核者", reviewer)]
                .into_iter()
                .map(|(role, outcome)| {
                    let (ordered, skipped) = match outcome {
                        RouteOutcome::Assigned { ordered, skipped } => (
                            ordered
                                .into_iter()
                                .map(name)
                                .collect::<Vec<_>>()
                                .join(" → "),
                            skipped,
                        ),
                        RouteOutcome::Escalate { skipped } => {
                            ("暂无可分配候选".to_owned(), skipped)
                        }
                    };
                    let reasons = skipped
                        .into_iter()
                        .map(|(id, reason)| {
                            format!("{}（{}）", name(id), skip_reason_label(reason))
                        })
                        .collect::<Vec<_>>()
                        .join("、");
                    div()
                        .flex()
                        .gap_3()
                        .border_t_1()
                        .border_color(theme.border)
                        .pt_2()
                        .child(div().w(px(66.0)).flex_shrink_0().child(role))
                        .child(div().min_w_0().flex_1().child(ordered))
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .text_color(theme.muted_foreground)
                                .child(if reasons.is_empty() {
                                    "暂无记录".to_owned()
                                } else {
                                    reasons
                                }),
                        )
                }),
        )
}

fn skip_reason_label(reason: SkipReason) -> &'static str {
    match reason {
        SkipReason::Disabled => "已停用",
        SkipReason::NoCapability => "缺少角色能力",
        SkipReason::SameFamily => "与执行者同家族",
        SkipReason::Unhealthy => "运行状态异常",
        SkipReason::LowQuota => "剩余额度不足",
    }
}
