//! `M_LoadDefaults`/`M_SaveDefaults` against `doomstat`: the config
//! values reach the live game state and come back out unchanged. (Own
//! test binary: it writes process-wide `doomstat` fields.)

use doommetal_rust::doomstat;
use doommetal_rust::m_misc::{m_load_defaults, m_save_defaults, Defaults};

#[test]
fn config_round_trips_through_the_game_state() {
    let dir = std::env::temp_dir().join(format!("doommetalrust_realcfg_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(".doomrc");

    std::fs::write(
        &path,
        "sfx_volume\t\t12\nmusic_volume 3\nmouse_sensitivity 9\nscreenblocks 11\nshow_messages 0\nusegamma 2\nsnd_channels 8\n",
    )
    .unwrap();

    let d = m_load_defaults(&path);
    let usegamma = d.apply();
    let st = doomstat::state();
    assert_eq!(usegamma, 2);
    assert_eq!(st.snd_sfx_volume, 12);
    assert_eq!(st.snd_music_volume, 3);
    assert_eq!(st.mouse_sensitivity, 9);
    assert_eq!(st.screenblocks, 11);
    assert_eq!(st.show_messages, 0);
    assert_eq!(st.num_channels, 8);
    // untouched entries keep the original table's defaults
    assert_eq!(st.detail_level, 0);

    // what the menu would change is what M_SaveDefaults writes back
    doomstat::state_mut().snd_sfx_volume = 5;
    doomstat::state_mut().screenblocks = 7;
    m_save_defaults(&path, &Defaults::capture(3));
    let again = m_load_defaults(&path);
    assert_eq!(again.get("sfx_volume"), Some(5));
    assert_eq!(again.get("screenblocks"), Some(7));
    assert_eq!(again.get("usegamma"), Some(3));
    assert_eq!(
        again.get("key_fire"),
        Some(doommetal_rust::doomdef::KEY_RCTRL)
    );

    std::fs::remove_dir_all(&dir).ok();
}
