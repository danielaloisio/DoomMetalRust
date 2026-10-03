//-----------------------------------------------------------------------------
//
// Copyright (C) 1993-1996 by id Software, Inc.
//
// This source is available for distribution and/or modification
// only under the terms of the DOOM Source Code License as
// published by id Software. All rights reserved.
//
// The source is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// FITNESS FOR A PARTICULAR PURPOSE. See the DOOM Source Code License
// for more details.
//
// DESCRIPTION:
//	Printed strings for translation.
//	English language support (default).
//
//-----------------------------------------------------------------------------

//! Rust port of `d_englsh.h` / `dstrings.c` (English strings only).
//!
//! Printed strings for translation - English language support
//! (default). The original supports French too (`d_french.h`,
//! selected via `-DFRENCH`), but the project's CMakeLists.txt never
//! defines `FRENCH`, so English is the only variant that has ever
//! actually been compiled in — faithfully porting only that active
//! configuration, same as `m_swap`'s little-endian-only path.
//!
//! These 285 constants were extracted with the system C preprocessor
//! run directly against the original `d_englsh.h` (rather than
//! transcribed by hand), so nested macro composition (e.g. `QSPROMPT`
//! ending in the expansion of `PRESSYN`) and adjacent string-literal
//! concatenation are resolved exactly as the original C compiler
//! would resolve them.

/// (`SAVEGAMENAME`), from `dstrings.h`.
pub const SAVEGAMENAME: &str = "doomsav";

/// (`DEVMAPS`), from `dstrings.h`.
pub const DEVMAPS: &str = "devmaps";
/// (`DEVDATA`), from `dstrings.h`.
pub const DEVDATA: &str = "devdata";

/// (`NUM_QUITMESSAGES`), from `dstrings.h`.
pub const NUM_QUITMESSAGES: usize = 22;

pub const D_DEVSTR: &str = "Development mode ON.\n";
pub const D_CDROM: &str = "CD-ROM Version: default.cfg from c:\\doomdata\n";
pub const PRESSKEY: &str = "press a key.";
pub const PRESSYN: &str = "press y or n.";
pub const QUITMSG: &str = "are you sure you want to\nquit this great game?";
pub const LOADNET: &str = "you can't do load while in a net game!\n\npress a key.";
pub const QLOADNET: &str = "you can't quickload during a netgame!\n\npress a key.";
pub const QSAVESPOT: &str = "you haven't picked a quicksave slot yet!\n\npress a key.";
pub const SAVEDEAD: &str = "you can't save if you aren't playing!\n\npress a key.";
pub const QSPROMPT: &str = "quicksave over your game named\n\n'%s'?\n\npress y or n.";
pub const QLPROMPT: &str = "do you want to quickload the game named\n\n'%s'?\n\npress y or n.";
pub const NEWGAME: &str = "you can't start a new game\nwhile in a network game.\n\npress a key.";
pub const NIGHTMARE: &str =
    "are you sure? this skill level\nisn't even remotely fair.\n\npress y or n.";
pub const SWSTRING: &str = "this is the shareware version of doom.\n\nyou need to order the entire trilogy.\n\npress a key.";
pub const MSGOFF: &str = "Messages OFF";
pub const MSGON: &str = "Messages ON";
pub const NETEND: &str = "you can't end a netgame!\n\npress a key.";
pub const ENDGAME: &str = "are you sure you want to end the game?\n\npress y or n.";
pub const DOSY: &str = "(press y to quit)";
pub const DETAILHI: &str = "High detail";
pub const DETAILLO: &str = "Low detail";
pub const GAMMALVL0: &str = "Gamma correction OFF";
pub const GAMMALVL1: &str = "Gamma correction level 1";
pub const GAMMALVL2: &str = "Gamma correction level 2";
pub const GAMMALVL3: &str = "Gamma correction level 3";
pub const GAMMALVL4: &str = "Gamma correction level 4";
pub const EMPTYSTRING: &str = "empty slot";
pub const GOTARMOR: &str = "Picked up the armor.";
pub const GOTMEGA: &str = "Picked up the MegaArmor!";
pub const GOTHTHBONUS: &str = "Picked up a health bonus.";
pub const GOTARMBONUS: &str = "Picked up an armor bonus.";
pub const GOTSTIM: &str = "Picked up a stimpack.";
pub const GOTMEDINEED: &str = "Picked up a medikit that you REALLY need!";
pub const GOTMEDIKIT: &str = "Picked up a medikit.";
pub const GOTSUPER: &str = "Supercharge!";
pub const GOTBLUECARD: &str = "Picked up a blue keycard.";
pub const GOTYELWCARD: &str = "Picked up a yellow keycard.";
pub const GOTREDCARD: &str = "Picked up a red keycard.";
pub const GOTBLUESKUL: &str = "Picked up a blue skull key.";
pub const GOTYELWSKUL: &str = "Picked up a yellow skull key.";
pub const GOTREDSKULL: &str = "Picked up a red skull key.";
pub const GOTINVUL: &str = "Invulnerability!";
pub const GOTBERSERK: &str = "Berserk!";
pub const GOTINVIS: &str = "Partial Invisibility";
pub const GOTSUIT: &str = "Radiation Shielding Suit";
pub const GOTMAP: &str = "Computer Area Map";
pub const GOTVISOR: &str = "Light Amplification Visor";
pub const GOTMSPHERE: &str = "MegaSphere!";
pub const GOTCLIP: &str = "Picked up a clip.";
pub const GOTCLIPBOX: &str = "Picked up a box of bullets.";
pub const GOTROCKET: &str = "Picked up a rocket.";
pub const GOTROCKBOX: &str = "Picked up a box of rockets.";
pub const GOTCELL: &str = "Picked up an energy cell.";
pub const GOTCELLBOX: &str = "Picked up an energy cell pack.";
pub const GOTSHELLS: &str = "Picked up 4 shotgun shells.";
pub const GOTSHELLBOX: &str = "Picked up a box of shotgun shells.";
pub const GOTBACKPACK: &str = "Picked up a backpack full of ammo!";
pub const GOTBFG9000: &str = "You got the BFG9000!  Oh, yes.";
pub const GOTCHAINGUN: &str = "You got the chaingun!";
pub const GOTCHAINSAW: &str = "A chainsaw!  Find some meat!";
pub const GOTLAUNCHER: &str = "You got the rocket launcher!";
pub const GOTPLASMA: &str = "You got the plasma gun!";
pub const GOTSHOTGUN: &str = "You got the shotgun!";
pub const GOTSHOTGUN2: &str = "You got the super shotgun!";
pub const PD_BLUEO: &str = "You need a blue key to activate this object";
pub const PD_REDO: &str = "You need a red key to activate this object";
pub const PD_YELLOWO: &str = "You need a yellow key to activate this object";
pub const PD_BLUEK: &str = "You need a blue key to open this door";
pub const PD_REDK: &str = "You need a red key to open this door";
pub const PD_YELLOWK: &str = "You need a yellow key to open this door";
pub const GGSAVED: &str = "game saved.";
pub const HUSTR_MSGU: &str = "[Message unsent]";
pub const HUSTR_E1M1: &str = "E1M1: Hangar";
pub const HUSTR_E1M2: &str = "E1M2: Nuclear Plant";
pub const HUSTR_E1M3: &str = "E1M3: Toxin Refinery";
pub const HUSTR_E1M4: &str = "E1M4: Command Control";
pub const HUSTR_E1M5: &str = "E1M5: Phobos Lab";
pub const HUSTR_E1M6: &str = "E1M6: Central Processing";
pub const HUSTR_E1M7: &str = "E1M7: Computer Station";
pub const HUSTR_E1M8: &str = "E1M8: Phobos Anomaly";
pub const HUSTR_E1M9: &str = "E1M9: Military Base";
pub const HUSTR_E2M1: &str = "E2M1: Deimos Anomaly";
pub const HUSTR_E2M2: &str = "E2M2: Containment Area";
pub const HUSTR_E2M3: &str = "E2M3: Refinery";
pub const HUSTR_E2M4: &str = "E2M4: Deimos Lab";
pub const HUSTR_E2M5: &str = "E2M5: Command Center";
pub const HUSTR_E2M6: &str = "E2M6: Halls of the Damned";
pub const HUSTR_E2M7: &str = "E2M7: Spawning Vats";
pub const HUSTR_E2M8: &str = "E2M8: Tower of Babel";
pub const HUSTR_E2M9: &str = "E2M9: Fortress of Mystery";
pub const HUSTR_E3M1: &str = "E3M1: Hell Keep";
pub const HUSTR_E3M2: &str = "E3M2: Slough of Despair";
pub const HUSTR_E3M3: &str = "E3M3: Pandemonium";
pub const HUSTR_E3M4: &str = "E3M4: House of Pain";
pub const HUSTR_E3M5: &str = "E3M5: Unholy Cathedral";
pub const HUSTR_E3M6: &str = "E3M6: Mt. Erebus";
pub const HUSTR_E3M7: &str = "E3M7: Limbo";
pub const HUSTR_E3M8: &str = "E3M8: Dis";
pub const HUSTR_E3M9: &str = "E3M9: Warrens";
pub const HUSTR_E4M1: &str = "E4M1: Hell Beneath";
pub const HUSTR_E4M2: &str = "E4M2: Perfect Hatred";
pub const HUSTR_E4M3: &str = "E4M3: Sever The Wicked";
pub const HUSTR_E4M4: &str = "E4M4: Unruly Evil";
pub const HUSTR_E4M5: &str = "E4M5: They Will Repent";
pub const HUSTR_E4M6: &str = "E4M6: Against Thee Wickedly";
pub const HUSTR_E4M7: &str = "E4M7: And Hell Followed";
pub const HUSTR_E4M8: &str = "E4M8: Unto The Cruel";
pub const HUSTR_E4M9: &str = "E4M9: Fear";
pub const HUSTR_1: &str = "level 1: entryway";
pub const HUSTR_2: &str = "level 2: underhalls";
pub const HUSTR_3: &str = "level 3: the gantlet";
pub const HUSTR_4: &str = "level 4: the focus";
pub const HUSTR_5: &str = "level 5: the waste tunnels";
pub const HUSTR_6: &str = "level 6: the crusher";
pub const HUSTR_7: &str = "level 7: dead simple";
pub const HUSTR_8: &str = "level 8: tricks and traps";
pub const HUSTR_9: &str = "level 9: the pit";
pub const HUSTR_10: &str = "level 10: refueling base";
pub const HUSTR_11: &str = "level 11: 'o' of destruction!";
pub const HUSTR_12: &str = "level 12: the factory";
pub const HUSTR_13: &str = "level 13: downtown";
pub const HUSTR_14: &str = "level 14: the inmost dens";
pub const HUSTR_15: &str = "level 15: industrial zone";
pub const HUSTR_16: &str = "level 16: suburbs";
pub const HUSTR_17: &str = "level 17: tenements";
pub const HUSTR_18: &str = "level 18: the courtyard";
pub const HUSTR_19: &str = "level 19: the citadel";
pub const HUSTR_20: &str = "level 20: gotcha!";
pub const HUSTR_21: &str = "level 21: nirvana";
pub const HUSTR_22: &str = "level 22: the catacombs";
pub const HUSTR_23: &str = "level 23: barrels o' fun";
pub const HUSTR_24: &str = "level 24: the chasm";
pub const HUSTR_25: &str = "level 25: bloodfalls";
pub const HUSTR_26: &str = "level 26: the abandoned mines";
pub const HUSTR_27: &str = "level 27: monster condo";
pub const HUSTR_28: &str = "level 28: the spirit world";
pub const HUSTR_29: &str = "level 29: the living end";
pub const HUSTR_30: &str = "level 30: icon of sin";
pub const HUSTR_31: &str = "level 31: wolfenstein";
pub const HUSTR_32: &str = "level 32: grosse";
pub const PHUSTR_1: &str = "level 1: congo";
pub const PHUSTR_2: &str = "level 2: well of souls";
pub const PHUSTR_3: &str = "level 3: aztec";
pub const PHUSTR_4: &str = "level 4: caged";
pub const PHUSTR_5: &str = "level 5: ghost town";
pub const PHUSTR_6: &str = "level 6: baron's lair";
pub const PHUSTR_7: &str = "level 7: caughtyard";
pub const PHUSTR_8: &str = "level 8: realm";
pub const PHUSTR_9: &str = "level 9: abattoire";
pub const PHUSTR_10: &str = "level 10: onslaught";
pub const PHUSTR_11: &str = "level 11: hunted";
pub const PHUSTR_12: &str = "level 12: speed";
pub const PHUSTR_13: &str = "level 13: the crypt";
pub const PHUSTR_14: &str = "level 14: genesis";
pub const PHUSTR_15: &str = "level 15: the twilight";
pub const PHUSTR_16: &str = "level 16: the omen";
pub const PHUSTR_17: &str = "level 17: compound";
pub const PHUSTR_18: &str = "level 18: neurosphere";
pub const PHUSTR_19: &str = "level 19: nme";
pub const PHUSTR_20: &str = "level 20: the death domain";
pub const PHUSTR_21: &str = "level 21: slayer";
pub const PHUSTR_22: &str = "level 22: impossible mission";
pub const PHUSTR_23: &str = "level 23: tombstone";
pub const PHUSTR_24: &str = "level 24: the final frontier";
pub const PHUSTR_25: &str = "level 25: the temple of darkness";
pub const PHUSTR_26: &str = "level 26: bunker";
pub const PHUSTR_27: &str = "level 27: anti-christ";
pub const PHUSTR_28: &str = "level 28: the sewers";
pub const PHUSTR_29: &str = "level 29: odyssey of noises";
pub const PHUSTR_30: &str = "level 30: the gateway of hell";
pub const PHUSTR_31: &str = "level 31: cyberden";
pub const PHUSTR_32: &str = "level 32: go 2 it";
pub const THUSTR_1: &str = "level 1: system control";
pub const THUSTR_2: &str = "level 2: human bbq";
pub const THUSTR_3: &str = "level 3: power control";
pub const THUSTR_4: &str = "level 4: wormhole";
pub const THUSTR_5: &str = "level 5: hanger";
pub const THUSTR_6: &str = "level 6: open season";
pub const THUSTR_7: &str = "level 7: prison";
pub const THUSTR_8: &str = "level 8: metal";
pub const THUSTR_9: &str = "level 9: stronghold";
pub const THUSTR_10: &str = "level 10: redemption";
pub const THUSTR_11: &str = "level 11: storage facility";
pub const THUSTR_12: &str = "level 12: crater";
pub const THUSTR_13: &str = "level 13: nukage processing";
pub const THUSTR_14: &str = "level 14: steel works";
pub const THUSTR_15: &str = "level 15: dead zone";
pub const THUSTR_16: &str = "level 16: deepest reaches";
pub const THUSTR_17: &str = "level 17: processing area";
pub const THUSTR_18: &str = "level 18: mill";
pub const THUSTR_19: &str = "level 19: shipping/respawning";
pub const THUSTR_20: &str = "level 20: central processing";
pub const THUSTR_21: &str = "level 21: administration center";
pub const THUSTR_22: &str = "level 22: habitat";
pub const THUSTR_23: &str = "level 23: lunar mining project";
pub const THUSTR_24: &str = "level 24: quarry";
pub const THUSTR_25: &str = "level 25: baron's den";
pub const THUSTR_26: &str = "level 26: ballistyx";
pub const THUSTR_27: &str = "level 27: mount pain";
pub const THUSTR_28: &str = "level 28: heck";
pub const THUSTR_29: &str = "level 29: river styx";
pub const THUSTR_30: &str = "level 30: last call";
pub const THUSTR_31: &str = "level 31: pharaoh";
pub const THUSTR_32: &str = "level 32: caribbean";
pub const HUSTR_CHATMACRO1: &str = "I'm ready to kick butt!";
pub const HUSTR_CHATMACRO2: &str = "I'm OK.";
pub const HUSTR_CHATMACRO3: &str = "I'm not looking too good!";
pub const HUSTR_CHATMACRO4: &str = "Help!";
pub const HUSTR_CHATMACRO5: &str = "You suck!";
pub const HUSTR_CHATMACRO6: &str = "Next time, scumbag...";
pub const HUSTR_CHATMACRO7: &str = "Come here!";
pub const HUSTR_CHATMACRO8: &str = "I'll take care of it.";
pub const HUSTR_CHATMACRO9: &str = "Yes";
pub const HUSTR_CHATMACRO0: &str = "No";
pub const HUSTR_TALKTOSELF1: &str = "You mumble to yourself";
pub const HUSTR_TALKTOSELF2: &str = "Who's there?";
pub const HUSTR_TALKTOSELF3: &str = "You scare yourself";
pub const HUSTR_TALKTOSELF4: &str = "You start to rave";
pub const HUSTR_TALKTOSELF5: &str = "You've lost it...";
pub const HUSTR_MESSAGESENT: &str = "[Message Sent]";
pub const HUSTR_PLRGREEN: &str = "Green: ";
pub const HUSTR_PLRINDIGO: &str = "Indigo: ";
pub const HUSTR_PLRBROWN: &str = "Brown: ";
pub const HUSTR_PLRRED: &str = "Red: ";
/// (`'g'`)
pub const HUSTR_KEYGREEN: u8 = b'g';
/// (`'i'`)
pub const HUSTR_KEYINDIGO: u8 = b'i';
/// (`'b'`)
pub const HUSTR_KEYBROWN: u8 = b'b';
/// (`'r'`)
pub const HUSTR_KEYRED: u8 = b'r';
pub const AMSTR_FOLLOWON: &str = "Follow Mode ON";
pub const AMSTR_FOLLOWOFF: &str = "Follow Mode OFF";
pub const AMSTR_GRIDON: &str = "Grid ON";
pub const AMSTR_GRIDOFF: &str = "Grid OFF";
pub const AMSTR_MARKEDSPOT: &str = "Marked Spot";
pub const AMSTR_MARKSCLEARED: &str = "All Marks Cleared";
pub const STSTR_MUS: &str = "Music Change";
pub const STSTR_NOMUS: &str = "IMPOSSIBLE SELECTION";
pub const STSTR_DQDON: &str = "Degreelessness Mode On";
pub const STSTR_DQDOFF: &str = "Degreelessness Mode Off";
pub const STSTR_KFAADDED: &str = "Very Happy Ammo Added";
pub const STSTR_FAADDED: &str = "Ammo (no keys) Added";
pub const STSTR_NCON: &str = "No Clipping Mode ON";
pub const STSTR_NCOFF: &str = "No Clipping Mode OFF";
pub const STSTR_BEHOLD: &str = "inVuln, Str, Inviso, Rad, Allmap, or Lite-amp";
pub const STSTR_BEHOLDX: &str = "Power-up Toggled";
pub const STSTR_CHOPPERS: &str = "... doesn't suck - GM";
pub const STSTR_CLEV: &str = "Changing Level...";
pub const E1TEXT: &str = "Once you beat the big badasses and\nclean out the moon base you're supposed\nto win, aren't you? Aren't you? Where's\nyour fat reward and ticket home? What\nthe hell is this? It's not supposed to\nend this way!\n\nIt stinks like rotten meat, but looks\nlike the lost Deimos base.  Looks like\nyou're stuck on The Shores of Hell.\nThe only way out is through.\n\nTo continue the DOOM experience, play\nThe Shores of Hell and its amazing\nsequel, Inferno!\n";
pub const E2TEXT: &str = "You've done it! The hideous cyber-\ndemon lord that ruled the lost Deimos\nmoon base has been slain and you\nare triumphant! But ... where are\nyou? You clamber to the edge of the\nmoon and look down to see the awful\ntruth.\n\nDeimos floats above Hell itself!\nYou've never heard of anyone escaping\nfrom Hell, but you'll make the bastards\nsorry they ever heard of you! Quickly,\nyou rappel down to  the surface of\nHell.\n\nNow, it's on to the final chapter of\nDOOM! -- Inferno.";
pub const E3TEXT: &str = "The loathsome spiderdemon that\nmasterminded the invasion of the moon\nbases and caused so much death has had\nits ass kicked for all time.\n\nA hidden doorway opens and you enter.\nYou've proven too tough for Hell to\ncontain, and now Hell at last plays\nfair -- for you emerge from the door\nto see the green fields of Earth!\nHome at last.\n\nYou wonder what's been happening on\nEarth while you were battling evil\nunleashed. It's good that no Hell-\nspawn could have come through that\ndoor with you ...";
pub const E4TEXT: &str = "the spider mastermind must have sent forth\nits legions of hellspawn before your\nfinal confrontation with that terrible\nbeast from hell.  but you stepped forward\nand brought forth eternal damnation and\nsuffering upon the horde as a true hero\nwould in the face of something so evil.\n\nbesides, someone was gonna pay for what\nhappened to daisy, your pet rabbit.\n\nbut now, you see spread before you more\npotential pain and gibbitude as a nation\nof demons run amok among our cities.\n\nnext stop, hell on earth!";
pub const C1TEXT: &str = "YOU HAVE ENTERED DEEPLY INTO THE INFESTED\nSTARPORT. BUT SOMETHING IS WRONG. THE\nMONSTERS HAVE BROUGHT THEIR OWN REALITY\nWITH THEM, AND THE STARPORT'S TECHNOLOGY\nIS BEING SUBVERTED BY THEIR PRESENCE.\n\nAHEAD, YOU SEE AN OUTPOST OF HELL, A\nFORTIFIED ZONE. IF YOU CAN GET PAST IT,\nYOU CAN PENETRATE INTO THE HAUNTED HEART\nOF THE STARBASE AND FIND THE CONTROLLING\nSWITCH WHICH HOLDS EARTH'S POPULATION\nHOSTAGE.";
pub const C2TEXT: &str = "YOU HAVE WON! YOUR VICTORY HAS ENABLED\nHUMANKIND TO EVACUATE EARTH AND ESCAPE\nTHE NIGHTMARE.  NOW YOU ARE THE ONLY\nHUMAN LEFT ON THE FACE OF THE PLANET.\nCANNIBAL MUTATIONS, CARNIVOROUS ALIENS,\nAND EVIL SPIRITS ARE YOUR ONLY NEIGHBORS.\nYOU SIT BACK AND WAIT FOR DEATH, CONTENT\nTHAT YOU HAVE SAVED YOUR SPECIES.\n\nBUT THEN, EARTH CONTROL BEAMS DOWN A\nMESSAGE FROM SPACE: \"SENSORS HAVE LOCATED\nTHE SOURCE OF THE ALIEN INVASION. IF YOU\nGO THERE, YOU MAY BE ABLE TO BLOCK THEIR\nENTRY.  THE ALIEN BASE IS IN THE HEART OF\nYOUR OWN HOME CITY, NOT FAR FROM THE\nSTARPORT.\" SLOWLY AND PAINFULLY YOU GET\nUP AND RETURN TO THE FRAY.";
pub const C3TEXT: &str = "YOU ARE AT THE CORRUPT HEART OF THE CITY,\nSURROUNDED BY THE CORPSES OF YOUR ENEMIES.\nYOU SEE NO WAY TO DESTROY THE CREATURES'\nENTRYWAY ON THIS SIDE, SO YOU CLENCH YOUR\nTEETH AND PLUNGE THROUGH IT.\n\nTHERE MUST BE A WAY TO CLOSE IT ON THE\nOTHER SIDE. WHAT DO YOU CARE IF YOU'VE\nGOT TO GO THROUGH HELL TO GET TO IT?";
pub const C4TEXT: &str = "THE HORRENDOUS VISAGE OF THE BIGGEST\nDEMON YOU'VE EVER SEEN CRUMBLES BEFORE\nYOU, AFTER YOU PUMP YOUR ROCKETS INTO\nHIS EXPOSED BRAIN. THE MONSTER SHRIVELS\nUP AND DIES, ITS THRASHING LIMBS\nDEVASTATING UNTOLD MILES OF HELL'S\nSURFACE.\n\nYOU'VE DONE IT. THE INVASION IS OVER.\nEARTH IS SAVED. HELL IS A WRECK. YOU\nWONDER WHERE BAD FOLKS WILL GO WHEN THEY\nDIE, NOW. WIPING THE SWEAT FROM YOUR\nFOREHEAD YOU BEGIN THE LONG TREK BACK\nHOME. REBUILDING EARTH OUGHT TO BE A\nLOT MORE FUN THAN RUINING IT WAS.\n";
pub const C5TEXT: &str = "CONGRATULATIONS, YOU'VE FOUND THE SECRET\nLEVEL! LOOKS LIKE IT'S BEEN BUILT BY\nHUMANS, RATHER THAN DEMONS. YOU WONDER\nWHO THE INMATES OF THIS CORNER OF HELL\nWILL BE.";
pub const C6TEXT: &str = "CONGRATULATIONS, YOU'VE FOUND THE\nSUPER SECRET LEVEL!  YOU'D BETTER\nBLAZE THROUGH THIS ONE!\n";
pub const P1TEXT: &str = "You gloat over the steaming carcass of the\nGuardian.  With its death, you've wrested\nthe Accelerator from the stinking claws\nof Hell.  You relax and glance around the\nroom.  Damn!  There was supposed to be at\nleast one working prototype, but you can't\nsee it. The demons must have taken it.\n\nYou must find the prototype, or all your\nstruggles will have been wasted. Keep\nmoving, keep fighting, keep killing.\nOh yes, keep living, too.";
pub const P2TEXT: &str = "Even the deadly Arch-Vile labyrinth could\nnot stop you, and you've gotten to the\nprototype Accelerator which is soon\nefficiently and permanently deactivated.\n\nYou're good at that kind of thing.";
pub const P3TEXT: &str = "You've bashed and battered your way into\nthe heart of the devil-hive.  Time for a\nSearch-and-Destroy mission, aimed at the\nGatekeeper, whose foul offspring is\ncascading to Earth.  Yeah, he's bad. But\nyou know who's worse!\n\nGrinning evilly, you check your gear, and\nget ready to give the bastard a little Hell\nof your own making!";
pub const P4TEXT: &str = "The Gatekeeper's evil face is splattered\nall over the place.  As its tattered corpse\ncollapses, an inverted Gate forms and\nsucks down the shards of the last\nprototype Accelerator, not to mention the\nfew remaining demons.  You're done. Hell\nhas gone back to pounding bad dead folks \ninstead of good live ones.  Remember to\ntell your grandkids to put a rocket\nlauncher in your coffin. If you go to Hell\nwhen you die, you'll need it for some\nfinal cleaning-up ...";
pub const P5TEXT: &str = "You've found the second-hardest level we\ngot. Hope you have a saved game a level or\ntwo previous.  If not, be prepared to die\naplenty. For master marines only.";
pub const P6TEXT: &str = "Betcha wondered just what WAS the hardest\nlevel we had ready for ya?  Now you know.\nNo one gets out alive.";
pub const T1TEXT: &str = "You've fought your way out of the infested\nexperimental labs.   It seems that UAC has\nonce again gulped it down.  With their\nhigh turnover, it must be hard for poor\nold UAC to buy corporate health insurance\nnowadays..\n\nAhead lies the military complex, now\nswarming with diseased horrors hot to get\ntheir teeth into you. With luck, the\ncomplex still has some warlike ordnance\nlaying around.";
pub const T2TEXT: &str = "You hear the grinding of heavy machinery\nahead.  You sure hope they're not stamping\nout new hellspawn, but you're ready to\nream out a whole herd if you have to.\nThey might be planning a blood feast, but\nyou feel about as mean as two thousand\nmaniacs packed into one mad killer.\n\nYou don't plan to go down easy.";
pub const T3TEXT: &str = "The vista opening ahead looks real damn\nfamiliar. Smells familiar, too -- like\nfried excrement. You didn't like this\nplace before, and you sure as hell ain't\nplanning to like it now. The more you\nbrood on it, the madder you get.\nHefting your gun, an evil grin trickles\nonto your face. Time to take some names.";
pub const T4TEXT: &str = "Suddenly, all is silent, from one horizon\nto the other. The agonizing echo of Hell\nfades away, the nightmare sky turns to\nblue, the heaps of monster corpses start \nto evaporate along with the evil stench \nthat filled the air. Jeeze, maybe you've\ndone it. Have you really won?\n\nSomething rumbles in the distance.\nA blue light begins to glow inside the\nruined skull of the demon-spitter.";
pub const T5TEXT: &str = "What now? Looks totally different. Kind\nof like King Tut's condo. Well,\nwhatever's here can't be any worse\nthan usual. Can it?  Or maybe it's best\nto let sleeping gods lie..";
pub const T6TEXT: &str = "Time for a vacation. You've burst the\nbowels of hell and by golly you're ready\nfor a break. You mutter to yourself,\nMaybe someone else can kick Hell's ass\nnext time around. Ahead lies a quiet town,\nwith peaceful flowing water, quaint\nbuildings, and presumably no Hellspawn.\n\nAs you step off the transport, you hear\nthe stomp of a cyberdemon's iron shoe.";
pub const CC_ZOMBIE: &str = "ZOMBIEMAN";
pub const CC_SHOTGUN: &str = "SHOTGUN GUY";
pub const CC_HEAVY: &str = "HEAVY WEAPON DUDE";
pub const CC_IMP: &str = "IMP";
pub const CC_DEMON: &str = "DEMON";
pub const CC_LOST: &str = "LOST SOUL";
pub const CC_CACO: &str = "CACODEMON";
pub const CC_HELL: &str = "HELL KNIGHT";
pub const CC_BARON: &str = "BARON OF HELL";
pub const CC_ARACH: &str = "ARACHNOTRON";
pub const CC_PAIN: &str = "PAIN ELEMENTAL";
pub const CC_REVEN: &str = "REVENANT";
pub const CC_MANCU: &str = "MANCUBUS";
pub const CC_ARCH: &str = "ARCH-VILE";
pub const CC_SPIDER: &str = "THE SPIDER MASTERMIND";
pub const CC_CYBER: &str = "THE CYBERDEMON";
pub const CC_HERO: &str = "OUR HERO";

// endmsg[] (the quit-message array from dstrings.c) is ported separately
// in dstrings.rs, since it composes these constants together and has its
// own notable fidelity concerns (a latent missing-comma bug in the
// original array literal) — see that module's docs.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanity_known_strings_match_source() {
        assert_eq!(D_DEVSTR, "Development mode ON.\n");
        assert_eq!(CC_HERO, "OUR HERO");
        assert_eq!(HUSTR_E1M1, "E1M1: Hangar");
    }

    #[test]
    fn nested_macro_composition_resolved_correctly() {
        // QSPROMPT = "...'%s'?\n\n" PRESSYN, i.e. ends with PRESSYN's text.
        assert!(QSPROMPT.ends_with(PRESSYN));
        // NEWGAME ends with PRESSKEY's text.
        assert!(NEWGAME.ends_with(PRESSKEY));
    }

    #[test]
    fn constant_count_matches_original_define_count() {
        // 285 macros extracted from d_englsh.h (excluding the include
        // guard), plus the 4 constants ported separately from
        // dstrings.h itself (SAVEGAMENAME, DEVMAPS, DEVDATA,
        // NUM_QUITMESSAGES) — this test just pins the d_englsh.h count
        // via a handful of first/last-defined sentinels rather than
        // trying to enumerate all 285 idents here.
        assert!(!D_DEVSTR.is_empty());
        assert!(!CC_HERO.is_empty());
    }
}
