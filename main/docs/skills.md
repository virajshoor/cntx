# Skills

Skills are reusable instructions for project knowledge, coding standards, workflows, and repeated prompts.

Cntx loads skills from:

- user config: `cntx config path` sibling directory `skills/`
- project config: `.cntx/skills/`

Create a user skill:

```bash
cntx skill new repo-standards "Apply repository coding and testing standards"
```

Project skills can be committed with a repository by adding YAML files under `.cntx/skills/`.

The skill system is intentionally data-first so future plugins can contribute skills without changing the core application.
