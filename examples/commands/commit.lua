return {
  api_version = 1,
  description = "Group staged changes into a validated commit plan",
  run = function(koru)
    local snapshot, snapshot_error = koru.git.snapshot()
    if snapshot_error then
      return { status = "unavailable", error = snapshot_error }
    end

    local sections = {}
    for _, change in ipairs(snapshot.changes) do
      sections[#sections + 1] = "\nID: " .. change.id
        .. "\nChange: " .. change.summary
        .. "\nDiff excerpt:\n" .. change.diff .. "\n"
    end
    local proposal = koru.ai.ask_json({
      prompt = "Group every staged change ID into one or more logical commits. Use each ID exactly once. Return Conventional Commit subjects (type(scope): subject) and a short rationale. Do not invent paths or Git commands.\nStaged changes:\n" .. table.concat(sections, "\n"),
      schema = "commit_plan",
      mode = "prompt_validate",
    })
    local plan = koru.git.validate(proposal)
    return {
      status = plan.status,
      snapshot_id = plan.snapshot_id,
      plan_id = plan.plan_id,
      change_count = plan.change_count,
      groups = plan.groups,
    }
  end,
}
