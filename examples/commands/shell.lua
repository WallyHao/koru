return {
  api_version = 1,
  description = "Propose and run one approved shell action",
  arguments = {
    { name = "task", type = "string", required = true, max_len = 8192 },
  },
  run = function(koru, args)
    local proposal = koru.ai.ask_json({
      prompt = "Propose one safe shell command for this task. Use the current working directory when appropriate. Return a short explanation, the exact shell script, and an existing working directory. Task: " .. args.task,
      schema = "shell_proposal",
      mode = "prompt_validate",
    })
    local result, err = koru.shell.script(proposal.script, {
      cwd = proposal.cwd,
      explanation = proposal.explanation,
    })
    if err then
      return { ok = false, error = err }
    end
    return {
      ok = result.exit_code == 0,
      explanation = proposal.explanation,
      exit_code = result.exit_code,
      signal = result.signal,
      stdout = result.stdout,
      stderr = result.stderr,
      stdout_truncated = result.stdout_truncated,
      stderr_truncated = result.stderr_truncated,
    }
  end,
}
