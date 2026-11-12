#define DAYZMAKE_TEST_NAME "dayzmake test"
#define DAYZMAKE_TEST_VERSION 1.0

class CfgPatches
{
	class dayzmake_test
	{
		units[] = {};
		weapons[] = {};
		requiredVersion = 0.1;
		requiredAddons[] =
		{
			"DZ_Data"
		};
	};
};

class CfgMods
{
	class dayzmake_test
	{
		dir = "dayzmake_test";
		picture = "";
		action = "";
		hideName = 1;
		hidePicture = 1;
		name = DAYZMAKE_TEST_NAME;
		credits = "dayzmake test harness";
		author = "dayzmake test harness";
		authorID = "0";
		version = "1.0";
		extra = 0;
		type = "mod";

		dependencies[] = { "Game", "World", "Mission" };

		class defs
		{
			class missionScriptModule
			{
				value = "";
				files[] =
				{
					"dayzmake_test/scripts/5_Mission"
				};
			};
		};
	};
};
