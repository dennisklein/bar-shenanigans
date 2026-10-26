-- Replacement for the engine's base content gamedata/system.lua.
--
-- BAR's unit def files run with this table as their fallback environment,
-- and the post-processing scripts call system.lowerkeys().

local system = {}

for k, v in pairs(_G) do
	system[k] = v
end

local function lowerkeys(t)
	local new = {}
	for k, v in pairs(t) do
		if type(v) == "table" then
			lowerkeys(v)
		end
		if type(k) == "string" then
			new[k:lower()] = v
		else
			new[k] = v
		end
	end
	for k in pairs(t) do
		t[k] = nil
	end
	for k, v in pairs(new) do
		t[k] = v
	end
	return t
end

system.lowerkeys = lowerkeys

return system
