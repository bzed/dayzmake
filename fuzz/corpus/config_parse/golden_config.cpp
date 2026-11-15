#include "inc\macros.hpp"

class CfgPatches
{
    class MODNAME
    {
        units[] = {};
        weapons[] = {"golden_a", "golden_b"};
        requiredAddons[] = {"DZ_Data"};
    };
};

class Base;
class CfgGolden
{
    class Parent
    {
        scale = SCALE(2,3);
        sum = __EVAL(1+2);
        ratio = 0.75;
        offset = -5;
        name = "parent";
    };
    class Child: Parent
    {
        name = "child";
        delete Obsolete;
        label = unquotedvalue;
#ifdef GOLDEN_FLAG
        mode = 1;
#else
        mode = 2;
#endif
    };
};
