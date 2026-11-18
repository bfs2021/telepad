# GitLab Issues API — Field Reference

Base URL pattern: `https://{gitlab-host}/api/v4/projects/{id}/issues`

This project uses a **self-hosted GitLab** — derive `{gitlab-host}` from the
`origin` remote URL (`.git/config`), not `gitlab.com`.

Project ID can be either:
- The numeric ID (e.g., `12345`)
- The URL-encoded path (e.g., `org%2Frepo`)

## Authentication

```
Header: PRIVATE-TOKEN: <your_token>
```

## Create Issue — POST /projects/{id}/issues

| Field | Type | Notes |
|---|---|---|
| `title` | string | **Required** |
| `description` | string | Markdown supported |
| `labels` | string | Comma-separated, e.g. `"bug,backend"` |
| `assignee_ids` | array[int] | GitLab user IDs |
| `milestone_id` | int | Optional |
| `due_date` | string | Format: `YYYY-MM-DD` |
| `weight` | int | GitLab EE only |

## Update Issue — PUT /projects/{id}/issues/{issue_iid}

Same fields as create. Use `state_event: "close"` to close an issue.

## Search Issues — GET /projects/{id}/issues

| Param | Notes |
|---|---|
| `state` | `opened`, `closed`, `all` |
| `search` | Searches title and description |
| `labels` | Filter by label |
| `assignee_id` | Filter by assignee |
| `order_by` | `created_at`, `updated_at` |
| `sort` | `asc`, `desc` |

## Response shape (issue object)

```json
{
  "id": 1,
  "iid": 1,
  "title": "...",
  "description": "...",
  "state": "opened",
  "labels": ["bug"],
  "web_url": "https://{gitlab-host}/org/repo/-/issues/1",
  "assignees": [{ "username": "...", "name": "..." }],
  "created_at": "2024-01-01T00:00:00Z"
}
```

## Issue Notes (comments) — GET/POST /projects/{id}/issues/{issue_iid}/notes

`GET` lists notes newest-last by default; `POST` with `body` (string) adds a
comment. Notes cannot be deleted via a normal token scope in most instance
configs — treat comments as permanent once posted.

## Merge Requests — POST /projects/{id}/merge_requests

| Field | Type | Notes |
|---|---|---|
| `source_branch` | string | **Required** |
| `target_branch` | string | **Required** |
| `title` | string | **Required** |
| `description` | string | Markdown supported |
| `remove_source_branch` | bool | Optional |

Update/read: `PUT`/`GET /projects/{id}/merge_requests/{merge_request_iid}`.
Key response fields: `has_conflicts`, `merge_status`/`detailed_merge_status`
(`can_be_merged`, `checking`, `preparing`, etc.), `state` (`opened`/`merged`/
`closed`), `should_remove_source_branch`.

## Merge — PUT /projects/{id}/merge_requests/{merge_request_iid}/merge

Merges the MR immediately if `has_conflicts` is `false` and
`detailed_merge_status` indicates it's mergeable. Optional fields:
`should_remove_source_branch` (bool), `squash` (bool),
`merge_commit_message` (string). Returns 405 if not currently mergeable
(pipeline still running, conflicts, etc.) — check status before calling.

## Common Errors

| Code | Meaning |
|---|---|
| 401 | Invalid or missing token |
| 403 | Token lacks `api` scope or user lacks project access |
| 404 | Project not found or no access |
| 405 | Merge request not currently mergeable (merge endpoint only) |
| 422 | Validation failed (e.g., duplicate title, invalid field) |