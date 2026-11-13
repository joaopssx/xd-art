-- Converts numbered frames (walk_01.png, walk_02.png, ...) and packs them into one
-- sprite sheet with the same palette for every frame.
--
--   xd-art-cli run sprite_sheet.lua <frames folder> <pattern> <sheet.png> [columns]
--   xd-art-cli run sprite_sheet.lua frames "walk_*.png" walk_sheet.png 4

local folder, pattern, sheet, columns = arg[1], arg[2], arg[3], tonumber(arg[4] or "0")
if not folder or not pattern or not sheet then
  error("usage: sprite_sheet.lua <frames folder> <pattern> <sheet.png> [columns]")
end

local settings = xd.preset("pico-8")
settings.sample = { mode = "box", width = 32, height = 32 }

local frames = {}
for _, path in ipairs(xd.files(folder, pattern)) do
  frames[#frames + 1] = xd.convert(xd.load(path), settings)
end
if #frames == 0 then
  error("no frames match " .. pattern .. " in " .. folder)
end
xd.sheet(frames, columns, 1):save(sheet)
print(#frames .. " frames -> " .. sheet)
