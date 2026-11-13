-- Renders one image with every dither mode into a folder, to pick one by eye.
--
--   xd-art-cli run compare_dithers.lua <image> <output folder> [width]

local input, output, width = arg[1], arg[2], tonumber(arg[3] or "64")
if not input or not output then
  error("usage: compare_dithers.lua <image> <output folder> [width]")
end

local modes = {
  "none", "bayer2x2", "bayer4x4", "bayer8x8", "cluster4x4", "floyd",
  "stucki", "sierra-lite", "picocad",
}
local img = xd.load(input)
for _, mode in ipairs(modes) do
  local out = xd.convert(img, {
    sample = { mode = "box", width = width, keep_aspect = true },
    dither = { mode = mode, amount = 0.5 },
    output = { scale = 4 },
  })
  out:save(output .. "/dither-" .. mode .. ".png")
  print(mode)
end
