-- Minimal stand-in for the Recoil engine's LuaParser environment.
--
-- BAR's gamedata scripts expect a few engine tables (VFS, Spring, Game,
-- Script, LOG). This file defines just enough of them to run
-- gamedata/unitdefs.lua outside the engine. The host provides the
-- __bar_* functions; they read from the BAR repository checkout.

local read, exists, dirlist, subdirs = __bar_read, __bar_exists, __bar_dirlist, __bar_subdirs
local embedded = __bar_embedded
__bar_read, __bar_exists, __bar_dirlist, __bar_subdirs, __bar_embedded = nil, nil, nil, nil, nil

LOG = { DEBUG = "debug", INFO = "info", NOTICE = "notice", WARNING = "warning", ERROR = "error", FATAL = "fatal" }

local warnings = {}
__bar_warnings = warnings

VFS = { RAW = "r", MOD = "M", MAP = "m", BASE = "b", ZIP = "Mmb", RAW_FIRST = "rMmb", ZIP_FIRST = "Mmbr" }

function VFS.FileExists(path)
	-- Models are not part of the data we need; treat every model as present
	-- so that unitdefs.lua does not drop units for a missing model file.
	if path:lower():find("^objects3d/") then
		return true
	end
	return exists(path) or embedded[path:lower()] ~= nil
end

function VFS.LoadFile(path)
	return read(path) or embedded[path:lower()]
end

function VFS.Include(path, env, _mode)
	local text = VFS.LoadFile(path)
	if text == nil then
		error("VFS.Include: file not found: " .. path, 2)
	end
	local chunk, err = loadstring(text, "@" .. path)
	if not chunk then
		error(err, 2)
	end
	setfenv(chunk, env or _G)
	return chunk()
end

function VFS.DirList(dir, pattern, _mode, recursive)
	return dirlist(dir, pattern or "*", recursive and true or false)
end

function VFS.SubDirs(dir, _pattern, _mode, _recursive)
	return subdirs(dir)
end

Script = {}

Game = {
	gameSpeed = 30,
	version = "bar-data",
	mapName = "",
	mapX = 0,
	mapY = 0,
	squareSize = 8,
	textColorCodes = { Color = "\255", ColorAndOutline = "\254", Reset = "\008" },
}

Engine = {
	version = "2025.06.0",
	versionFull = "2025.06.0 bar-data",
	versionMajor = "2025",
	versionMinor = "06",
	versionPatchSet = "0",
	commitsNumber = "0",
	FeatureSupport = {},
}

Spring = {}

function Spring.GetModOptions()
	return {}
end

function Spring.Echo(...)
	local parts = {}
	for i = 1, select("#", ...) do
		parts[#parts + 1] = tostring((select(i, ...)))
	end
	warnings[#warnings + 1] = table.concat(parts, " ")
end

function Spring.Log(section, level, ...)
	if level == LOG.ERROR or level == LOG.FATAL or level == LOG.WARNING then
		Spring.Echo("[" .. tostring(section) .. "] " .. tostring(level) .. ":", ...)
	end
end

function Spring.TimeCheck(_, fn)
	return fn()
end

function Spring.GetTeamList()
	return {}
end

function Spring.GetTeamLuaAI()
	return nil
end

function Spring.GetConfigInt(_, default)
	return default
end

function Spring.GetConfigString(_, default)
	return default
end

-- An empty game: no teams, no players. Game type checks see a plain
-- player-versus-player game without Raptors or Scavengers.
function Spring.GetGaiaTeamID()
	return 0
end

function Spring.GetTeamInfo(teamID)
	return teamID, 0, false, false, "", 0
end

function Spring.GetAllyTeamList()
	return {}
end

function Spring.GetPlayerList()
	return {}
end
