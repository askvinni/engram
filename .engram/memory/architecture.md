# architecture

- CLAUDE.md should hold only structural pointers (@path refs); all learning content lives exclusively in .engram/memory/ category files. _(from #8)
- The canonical engram prerequisite set is: git repo present, gh installed, gh authenticated, claude installed, .engram/config.toml exists, github repo configured — doctor validates all six. _(from #9)
- Resolve the GitHub repo in every command by checking config first then falling back to `infer_repo()` — never assume the repo is available without this two-step lookup. _(from #10)
- Multi-step workflow commands (land = learn → close issue → delete branch) should print a status line after each step so partial completion is visible if a later step fails. _(from #11)
