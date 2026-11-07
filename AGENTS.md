# Product requirements

- This is an end-user Canvas LMS client, not a developer tool or MCP explorer.
- Keep navigation familiar to Canvas users: Dashboard course cards, named course
  selection, course Home/Modules/Assignments/Grades/Files/Syllabus, and Calendar.
- Never require users to type course/assignment/file IDs, page slugs, resource
  URIs, tool names, or JSON. Carry identifiers internally from selected objects.
- canvas-mcp is a reference only. No build/runtime dependency on its executable,
  source checkout, settings, cookies, or browser profile. The app owns its REST
  client, authentication profile, planner operations, and document handling.

# Verification safety

- Do not send keyboard or pointer input to the shared display `:0`.
- Do not run shared-desktop automation. Use an isolated GPU-capable display if
  one is available; otherwise rely on compilation and automated non-UI tests.
- Xvfb without DRI3 does not support this GPUI/Vulkan setup. Do not substitute
  shared-desktop input when isolated visual testing is unavailable.
- Distinguish build/model/local-HTTP tests from real institutional-login and
  live-account verification. Never test uncertain-write retries on the account.
