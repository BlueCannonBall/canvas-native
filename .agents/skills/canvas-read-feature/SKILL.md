---
name: canvas-read-feature
description: Add a read-only Canvas surface (announcements, discussions, pages, or similar) to Canvas Native across the REST client, in-process operation catalog, routing, virtualized UI, sample account, and tests. Use when extending this app with new Canvas content rather than changing authentication, downloads, or planner writes.
---

# Add a read-only Canvas surface

Canvas Native is layered and every layer has a fixed place. Add the smallest
operation set that the screen needs, keep identifiers internal, and verify with
offline tests before claiming live Canvas behavior.

## Prerequisites

- Read `AGENTS.md`; never send input to display `:0`.
- Use the existing release test and isolated X11 commands from `README.md`.
- Confirm the Canvas REST endpoint and response fields. For announcements, the
  list is `/api/v1/courses/:id/discussion_topics?only_announcements=true` and
  the detail is `/api/v1/courses/:id/discussion_topics/:topic_id`.

## Procedure

1. **Define routing and requests first.** `src/model.rs`:
   - Add the course tab or `Route` variant.
   - Map the route to its request name and argument object in `requests`.
   - Add `course_id` coverage for every route that belongs to a course.
   - Add a `planner_route` mapping only when the planner item type is explicit
     and stable (for example `announcement`), not for ambiguous module types.
2. **Add the REST method and typed response.** `src/canvas_api.rs`:
   - Validate numeric IDs before building the path.
   - Use `api_get` or `api_get_paginated`; list calls get `per_page=100`.
   - Reuse `Attachment` and call `prepare_resources` for any returned files.
   - Keep Canvas field names available to the UI through serde renames.
3. **Expose the operation in-process.** `src/engine.rs`:
   - Add parameter structs, fetch methods, and dispatch arms.
   - Add the names to `OPERATION_NAMES` in `src/native_results.rs`.
   - Mark read operations cacheable in `src/backend.rs::is_cacheable_read`.
   - Update count assertions in engine, native-results, and backend tests.
4. **Prepare virtualized rows.** `src/content_rows.rs`:
   - Add a `ContentRow` variant and a builder that filters by the search query.
   - Sort with parsed timestamps, not raw string comparison, when order matters.
   - Keep the renderer in `src/views.rs` free of parsing and sorting work.
5. **Wire the screen.** `src/views.rs` and `src/app.rs`:
   - Add the course nav entry and mark the tab active for its detail route.
   - Add loading labels, `page_title` coverage, and the `Route` render arm.
   - Add `showing_content_rows` coverage for list screens so the pinned heading
     forwards wheel input to the list and the page does not scroll.
   - Omit the pinned heading/search for detail routes like assignments and pages.
   - Reuse `canvas_body`, `attachment_row`, `route_link`, and `web_link`.
6. **Update the sample account.** `src/demo.rs`:
   - Return distinct data per course and support both list and detail.
   - Add a deterministic `CANVAS_NATIVE_DEMO_PAGE` entry when useful for checks.
7. **Document and test.** Update `README.md` feature coverage and operation
   counts. Add tests for request mapping, parsing, row ordering/filtering,
   sample-data scoping, and a headless render test for a screen with a Canvas
   HTML body.

## Verification

```sh
cargo fmt --check
cargo test --release
cargo clippy --all-targets -- -D warnings
python3 scripts/test-isolated-x11.py
cargo build --release
env -u DISPLAY -u WAYLAND_DISPLAY target/release/canvas-native --check-backend
```

Check that `--check-backend` lists the new operations. Offline tests, local HTTP
fixtures, and the GPUI test platform do not prove live institutional login,
Canvas permissions, pagination against a real course, or visual layout.

## Pitfalls

- Do not add a new cacheable read without also adding it to the backend barrier
  list; otherwise it runs serialized or is mistaken for a write.
- Do not prefetch every course. The open-course prefetch is intentionally limited
  to Home, Modules, and Assignments metadata and has a test asserting that.
- Announcements are discussion topics. A module item of type `Discussion` may be
  either an announcement or a regular discussion; do not map it to the
  announcement detail screen without a reliable discriminator.
- Detail routes need `course_id` so the breadcrumb, course nav, and error state
  remain correct.
- Keep the detail page's body in `canvas_body` so Canvas file links route to the
  native reader rather than the system browser.
