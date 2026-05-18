# tripwires

- Inlining memory content into CLAUDE.md is unsafe: if any learning contains the engram end-marker string, it corrupts the section boundary — this already happened once. _(from #8)
- Invoking `claude -p` from within a repo directory causes CLAUDE.md to be loaded, turning a simple synthesis call into an action-taking agent — always set `current_dir(temp_dir())` for programmatic Claude calls that should only return structured output. _(from #26)
- Globbing a prompt-hooks directory loads README.md as a rule file unless explicitly filtered — always skip files by documentation naming convention (README.md, etc.) when loading prompt customization files. _(from #26)
