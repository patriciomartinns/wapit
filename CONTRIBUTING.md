# Contributing

Thank you for helping make wapit better. The most useful thing you can send is an issue.

## Issues, not pull requests

wapit does not accept pull requests, and the repository only lets the maintainer open them. Every change is written by the maintainer, with the issue as its specification. This keeps one person accountable for each line, keeps the code consistent, and spares you the work of a patch that might not fit. If you already wrote a fix in a fork, link it from the issue and it will be read as part of the report.

## A good bug report

- What your app sent: the request path and JSON body, with tokens removed.
- What wapit answered, and what Meta answers for the same request, with a link to Meta's documentation when you have one.
- The wapit version (`wapit --version`) and how you run it (Homebrew, Docker, source).

wapit's value is fidelity to the real Cloud API, so a report that shows a difference from Meta's behavior is the best kind.

## A good feature request

Describe the problem in your app first, then the endpoint or inbox action that would solve it. Requests for Graph API endpoints are judged by how many apps use them in development.

## Security

Do not open a public issue for a vulnerability. Follow [SECURITY.md](SECURITY.md).
