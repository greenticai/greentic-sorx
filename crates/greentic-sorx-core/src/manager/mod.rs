mod cards;
mod channel;
mod context;
mod filter;
mod locale;
mod policy;
mod view_model;

pub use cards::{
    render_dashboard_card, render_record_create_card, render_record_detail_card,
    render_record_list_card, render_record_picker_card, render_relationship_summary_card,
};
pub use channel::{ChannelCapabilities, ManagerChannel};
pub use context::{ManagerContextDefaults, SorxManagerContext, resolve_manager_context};
pub use filter::{ManagerPolicySet, filter_manager_view};
pub use locale::{
    ManagerLocaleBundle, ManagerLocaleCatalog, ManagerLocaleContext, TextDirection,
    format_manager_value, humanize_identifier, localize_manager_view,
};
pub use policy::{ManagerPolicyDecision, ManagerPolicyEffect};
pub use view_model::{
    ManagerActionView, ManagerFieldRelationshipView, ManagerFieldView, ManagerNavItem,
    ManagerPolicyHint, ManagerRecordView, ManagerRelationshipView, ManagerViewModel,
    generate_manager_view,
};
