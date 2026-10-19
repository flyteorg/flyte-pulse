@Copilot please review the proposed company and affiliation changes in this pull request and push fixes to this branch.

New companies in `config/companies.yaml`:
- Confirm that each company exists and use its official name as `canonical_name`.
- Set `website_url` to its official HTTPS website, and add other names it is known by as `aliases`.
- Check that new email `domains` belong to that company.

Affiliations in `config/affiliations.yaml`:
- Confirm that each GitHub account belongs to a real person, not a bot, an organization, or a shared account. Add bots to `identity_rules` in `config/repositories.yaml` with `kind: BOT` instead of an affiliation.
- Check that the evidence in the description (profile company, public organizations, commit email domains) supports the company and the dates. Dates are first and last observations, not employment dates; keep them unless you find better public evidence.

Remove entries you cannot support, and explain each removal in a comment. Keep the existing YAML layout and comments.
