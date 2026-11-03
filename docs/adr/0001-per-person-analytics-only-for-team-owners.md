# Per-person analytics only for team owners

Every team member can already read every issue in their team through the Linear API, so hiding data is not a security boundary. I still split team analytics in two: aggregate metrics are shown to every member, and the per-person breakdown (who closed what, how long each person takes) is shown only for teams the viewer can administer, read from `Query.administrableTeams`. A public tool that ranks individuals by default invites misuse.

## Considered options

- **Gate all team analytics on ownership**: rejected, because aggregates are harmless and useful to every member.
- **Show everything to everyone**: rejected for the reason above.
- **Use `TeamMembership.owner`**: rejected as the only check, because workspace admins own every team implicitly and that may not show up in the membership row; `administrableTeams` reflects effective permissions.

## Consequences

On Linear's Free plan every member is an admin, so every viewer sees the per-person breakdown there. This is a presentation rule, not access control: anyone can compute the same numbers from the API.
