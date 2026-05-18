# patterns

- Use the GraphQL CLOSED_EVENT timeline item (closer field) to find the PR that closed an issue — avoids post-merge search index lag that makes text search unreliable. _(from #5)
- Filter GraphQL PR results by state == MERGED rather than trusting the query to return only merged PRs — defensive against edge cases like closed-without-merge. _(from #5)
