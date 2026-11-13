-- Converts every PNG in a folder with one recipe.
--
--   xd-art-cli run convert_folder.lua <input folder> <output folder> [recipe.json]
--
-- The recipe is optional; without it the defaults are used.

local input, output, recipe = arg[1], arg[2], arg[3]
if not input or not output then
  error("usage: convert_folder.lua <input folder> <output folder> [recipe.json]")
end

local start = xd.time()
local count, failed = 0, 0
for _, path in ipairs(xd.files(input, "*.png")) do
  -- File name after the last / or \ (a long string, so the backslash stays literal).
  local name = path:match([[([^/\]+)$]])
  local ok, err = xd.process(path, output .. "/" .. name, recipe)
  if ok then
    count = count + 1
  else
    failed = failed + 1
    print("failed: " .. name .. ": " .. err)
  end
end
print(string.format("%d converted, %d failed in %.2f s", count, failed, xd.time() - start))
