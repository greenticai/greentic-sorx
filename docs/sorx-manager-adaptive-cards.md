# Sorx Manager Adaptive Cards

SORX emits canonical Adaptive Card JSON for manager views. The payloads are provider-neutral and do not include Slack Block Kit, Webex card transformation logic, or channel-specific rendering branches.

Current card renderers:

```text
render_dashboard_card
render_record_list_card
render_record_create_card
render_record_detail_card
render_relationship_summary_card
```

Cards include manager metadata under `metadata.schema = greentic.sorx.manager-card.v1`.

## SVG

`GET /v1/sorx/manager/graph.svg` is a convenience diagnostic. Adaptive Cards must not depend on clickable SVG behavior. Relationship cards should expose textual summaries and drill-down actions instead.

## Submit

Cards may submit record/action intent, but `POST /v1/sorx/manager/submit` re-resolves actor context and invokes the normal runtime path. The submitted card payload is never treated as proof of permission.
