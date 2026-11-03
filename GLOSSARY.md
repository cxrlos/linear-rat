# linear-rat

A terminal client for Linear. These are the words I use for the things lrat shows and acts on.

## People and access

**Viewer**:
The person running lrat, identified by the API key in use.
_Avoid_: user (overloaded with Linear's `User` type for every member), me, account

**Workspace**:
The single Linear organization the API key belongs to. lrat works with one workspace at a time.
_Avoid_: organization, org, tenant

**Team**:
A Linear team inside the workspace; issues, workflow states, labels and cycles belong to it.

**Team owner**:
A viewer who can administer a team, either as an explicit team owner or as a workspace admin (admins own every team they can access).
_Avoid_: team admin, manager, lead

## Work

**Issue**:
A unit of work in a team, identified by its key (for example `ENG-123`).
_Avoid_: ticket, task, card

**Workflow state**:
A team's named status for an issue (for example "In Review"), always belonging to one state type: triage, backlog, unstarted, started, completed or canceled.
_Avoid_: status, column

**Cycle**:
A team's fixed time box of committed issues.
_Avoid_: sprint, iteration

## Analytics

**Personal analytics**:
Metrics about the viewer's own issues.

**Team analytics**:
Aggregate metrics about a team's issues, visible to every team member.

**Per-person breakdown**:
Team analytics split by assignee, visible only to team owners.
_Avoid_: leaderboard, ranking

**Cycle time**:
Time from an issue first entering a started state to it entering a completed state.

**Lead time**:
Time from an issue's creation to it entering a completed state.

**Throughput**:
Number of issues entering a completed state per time bucket.

**Net flow**:
Issues created minus issues completed in a time bucket; positive means the backlog is growing.
_Avoid_: burn rate

**Velocity**:
Estimate points completed in a cycle.
_Avoid_: speed, capacity

**Scope creep**:
Estimate points added to a cycle after it started.

**Issue age**:
Time an open issue has spent in its current started state.
_Avoid_: staleness (see stale issue)

**Stale issue**:
An open issue with no update for a configured number of days.

**Triage time**:
Time from an issue's creation to it leaving triage.
