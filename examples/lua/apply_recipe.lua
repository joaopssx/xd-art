-- Applies a recipe file, but lets the script pick the width from the image: small
-- images keep more of their pixels, large ones go down to 128.
--
--   xd-art-cli run apply_recipe.lua <recipe.json> <image> <output.png>

local recipe, input, output = arg[1], arg[2], arg[3]
if not recipe or not input or not output then
  error("usage: apply_recipe.lua <recipe.json> <image> <output.png>")
end

local settings = xd.recipe(recipe)
local img = xd.load(input)
local width = math.min(128, math.max(16, img:width() // 4))
settings.sample = settings.sample or {}
settings.sample.width = width
settings.sample.keep_aspect = true
xd.convert(img, settings):save(output)
print(string.format("%s: %dx%d -> width %d", input, img:width(), img:height(), width))
