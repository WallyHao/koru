return {
  api_version = 1,
  description = "Ask the selected model a question",
  arguments = {
    { name = "prompt", type = "string", required = true, max_len = 8192 },
  },
  run = function(koru, args)
    local result = koru.ai.ask(args.prompt)
    return result.text
  end,
}
