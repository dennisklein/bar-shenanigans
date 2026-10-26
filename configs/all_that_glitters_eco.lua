-- eco-opener run config: best Armada eco at minute 5 on All That Glitters.
-- Paths are relative to this file. bar_root is a checkout of
-- https://github.com/beyond-all-reason/Beyond-All-Reason next to this repository.
return {
	bar_root = "../../Beyond-All-Reason",
	map = "../data/maps/all_that_glitters.lua",
	start = "P1",
	commander = "armcom",

	-- Units that a plan may order, besides the commander. Each must be
	-- buildable from the commander through these units.
	units = {
		"armmex", -- Metal Extractor
		"armsolar", -- Solar Collector
		"armwin", -- Wind Turbine
		"armmakr", -- Energy Converter
		"armadvsol", -- Advanced Solar Collector
		"armnanotc", -- Construction Turret
		"armlab", -- Bot Lab
		"armvp", -- Vehicle Plant
		"armap", -- Aircraft Plant
		"armck", -- Construction Bot
		"armcv", -- Construction Vehicle
		"armca", -- Construction Aircraft
	},

	-- "nearest_start": the spots nearer to this start than to any other
	-- start position (your share in a full 8v8); "all": every spot.
	metal_spots = "nearest_start",

	horizon_s = 300,

	-- "max_eco": metal income + net energy income / 70 at the horizon.
	-- Or Lua source that returns function(o) return score end, with
	-- o.metal_income, o.energy_income, o.energy_upkeep, o.metal, o.energy,
	-- o.build_power, o.metal_wasted, o.energy_wasted, o.horizon_s.
	objective = "max_eco",

	search = { iterations = 60000, runs = 8, seed = 1 },
	sim = { dt = 0.1 },
}
