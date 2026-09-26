use dreamchecker::test_helpers::{
    sleep_allowlist_for_test, sleep_allowlist_trusting_new_of_variable_for_test,
    sleep_allowlist_trusting_unresolved_for_test,
};

fn admitted(allowlist: &[String], path: &str) -> bool {
    allowlist.iter().any(|each| each == path)
}

#[test]
fn a_waitfor_zero_callee_protects_its_caller_but_not_itself() {
    let code = r##"
/proc/fire()
    sleep(1)
/proc/ignite()
    set waitfor = 0
    fire()
/proc/run_queue()
    ignite()
"##
    .trim();
    for version in [1, 3] {
        let allowlist = sleep_allowlist_for_test(code, version);
        assert!(admitted(&allowlist, "/proc/run_queue"), "v{version}");
        assert!(!admitted(&allowlist, "/proc/ignite"), "v{version}");
        assert!(!admitted(&allowlist, "/proc/fire"), "v{version}");
    }
}

#[test]
fn calls_it_cannot_resolve_count_as_sleeping() {
    let code = r##"
/datum/proc/quiet()
/proc/untyped(x)
    x.quiet()
/proc/colon(datum/x)
    x:quiet()
/proc/typed(datum/x)
    x.quiet()
"##
    .trim();
    for version in [1, 3] {
        let allowlist = sleep_allowlist_for_test(code, version);
        assert!(admitted(&allowlist, "/proc/typed"), "v{version}");
        assert!(!admitted(&allowlist, "/proc/untyped"), "v{version}");
        assert!(!admitted(&allowlist, "/proc/colon"), "v{version}");
    }
}

#[test]
fn trusting_unresolved_calls_admits_them_but_not_real_sleeps() {
    let code = r##"
/datum/proc/quiet()
/proc/untyped(x)
    x.quiet()
/proc/sleeper(x)
    x.quiet()
    sleep(1)
"##
    .trim();
    let allowlist = sleep_allowlist_trusting_unresolved_for_test(code, 3);
    assert!(admitted(&allowlist, "/proc/untyped"));
    assert!(!admitted(&allowlist, "/proc/sleeper"));
}

#[test]
fn new_of_a_typed_variable_checks_that_types_new() {
    let code = r##"
/datum/quiet/New()
/datum/quiet/child/New()
    sleep(1)
/datum/loud/New()
    sleep(1)
/proc/makes_quiet(datum/quiet/T)
    new T()
/proc/makes_loud(datum/loud/T)
    new T()
/proc/hinted(T)
    var/datum/quiet/Q = new T()
/proc/untyped(T)
    new T()
"##
    .trim();
    let v3 = sleep_allowlist_for_test(code, 3);
    assert!(admitted(&v3, "/proc/makes_quiet"));
    assert!(admitted(&v3, "/proc/hinted"));
    assert!(!admitted(&v3, "/proc/makes_loud"));
    assert!(!admitted(&v3, "/proc/untyped"));
    // v1 follows every override, and T may hold /datum/quiet/child
    assert!(!admitted(
        &sleep_allowlist_for_test(code, 1),
        "/proc/makes_quiet"
    ));
}

#[test]
fn trusting_new_of_variable_admits_only_that() {
    let code = r##"
/datum/proc/quiet()
/proc/sleeper()
    sleep(1)
/proc/untyped_new(T)
    new T()
/proc/untyped_call(x)
    x.quiet()
/proc/new_with_sleeping_arg(T)
    new T(sleeper())
"##
    .trim();
    let allowlist = sleep_allowlist_trusting_new_of_variable_for_test(code, 3);
    assert!(admitted(&allowlist, "/proc/untyped_new"));
    assert!(!admitted(&allowlist, "/proc/untyped_call"));
    assert!(!admitted(&allowlist, "/proc/new_with_sleeping_arg"));
}

#[test]
fn a_sleep_inside_spawn_does_not_park_the_spawner() {
    let code = r##"
/proc/fire()
    sleep(1)
/proc/spawner()
    spawn(1)
        fire()
        call(/proc/fire)()
"##
    .trim();
    assert!(admitted(
        &sleep_allowlist_for_test(code, 3),
        "/proc/spawner"
    ));
}

#[test]
fn allowed_to_sleep_still_sleeps() {
    let code = r##"
/proc/exempt()
    set SpacemanDMM_allowed_to_sleep = TRUE
    sleep(1)
/proc/caller_of_exempt()
    exempt()
"##
    .trim();
    let allowlist = sleep_allowlist_for_test(code, 3);
    assert!(!admitted(&allowlist, "/proc/exempt"));
    assert!(!admitted(&allowlist, "/proc/caller_of_exempt"));
}

#[test]
fn overrides_are_followed_through_every_level() {
    let code = r##"
/mob/proc/foo()
/mob/proc/wait()
    sleep(1)
/mob/living/carbon/foo()
    wait()
/mob/living/proc/bar()
    foo()
/mob/living/proc/unrelated()
"##
    .trim();
    for version in [1, 3] {
        let allowlist = sleep_allowlist_for_test(code, version);
        assert!(!admitted(&allowlist, "/mob/living/proc/bar"), "v{version}");
        assert!(
            admitted(&allowlist, "/mob/living/proc/unrelated"),
            "v{version}"
        );
    }
}

#[test]
fn version_3_only_follows_overrides_under_the_receiver() {
    let code = r##"
/mob/proc/foo()
/mob/living/proc/bar()
    foo()
/mob/dead/foo()
    sleep(1)
"##
    .trim();
    assert!(!admitted(
        &sleep_allowlist_for_test(code, 1),
        "/mob/living/proc/bar"
    ));
    assert!(admitted(
        &sleep_allowlist_for_test(code, 3),
        "/mob/living/proc/bar"
    ));
}

#[test]
fn version_3_keeps_the_receiver_through_a_base_type_proc() {
    // the /turf/open/process_cell -> /atom/proc/update_appearance shape: the
    // self-call inside the /atom proc still runs on a /obj/machine
    let code = r##"
/atom/proc/update_appearance()
    update_icon()
/atom/proc/update_icon()
/obj/machine/proc/set_on()
    update_appearance()
/mob/update_icon()
    sleep(1)
"##
    .trim();
    let v1 = sleep_allowlist_for_test(code, 1);
    let v3 = sleep_allowlist_for_test(code, 3);
    assert!(!admitted(&v1, "/obj/machine/proc/set_on"));
    assert!(admitted(&v3, "/obj/machine/proc/set_on"));
    assert!(!admitted(&v3, "/atom/proc/update_appearance"));
}

#[test]
fn signal_dispatch_does_not_count_as_sleeping() {
    let code = r##"
/datum/proc/_SendSignal(sigtype, list/arguments)
    return call(src, sigtype)(arglist(arguments))
/datum/proc/sender()
    _SendSignal("x", list())
"##
    .trim();
    for version in [1, 3] {
        let allowlist = sleep_allowlist_for_test(code, version);
        assert!(admitted(&allowlist, "/datum/proc/sender"), "v{version}");
    }
}

#[test]
fn every_builtin_that_parks_counts() {
    let code = r##"
/proc/quiet()
/proc/starts()
    startup("x.dmb", 0)
/proc/shuts(addr)
    shutdown(addr)
/proc/medal()
    world.GetMedal("m", "k")
/proc/page(client/C)
    C.SendPage("hi", "k")
/proc/icon(client/C)
    C.RenderIcon(null)
/proc/opens()
    var/savefile/S = new /savefile("x.sav")
/proc/locks(savefile/S)
    S.Lock(1)
/proc/background_loop()
    set background = 1
    for(var/i in 1 to 10)
        quiet()
"##
    .trim();
    let allowlist = sleep_allowlist_for_test(code, 3);
    assert!(admitted(&allowlist, "/proc/quiet"));
    let wrongly_admitted: Vec<_> = [
        "/proc/starts",
        "/proc/shuts",
        "/proc/medal",
        "/proc/page",
        "/proc/icon",
        "/proc/opens",
        "/proc/locks",
        "/proc/background_loop",
    ]
    .into_iter()
    .filter(|path| admitted(&allowlist, path))
    .collect();
    assert!(wrongly_admitted.is_empty(), "{wrongly_admitted:?}");
}

// dm.exe 516.1687 keeps proc/ or verb/ only on the copy that declares the proc:
// /proc/twice then /twice, /datum/proc/quiet then /datum/quiet
#[test]
fn a_redefinition_on_the_declaring_type_is_spelled_without_proc() {
    let code = r##"
/proc/twice()
/twice()
    ..()
/datum/proc/quiet()
/datum/quiet()
    ..()
/datum/proc/loud()
/datum/loud()
    sleep(1)
"##
    .trim();
    for version in [1, 3] {
        let allowlist = sleep_allowlist_for_test(code, version);
        assert!(admitted(&allowlist, "/proc/twice"), "v{version}");
        assert!(admitted(&allowlist, "/twice"), "v{version}");
        assert!(admitted(&allowlist, "/datum/proc/quiet"), "v{version}");
        assert!(admitted(&allowlist, "/datum/quiet"), "v{version}");
        assert!(admitted(&allowlist, "/datum/proc/loud"), "v{version}");
        assert!(!admitted(&allowlist, "/datum/loud"), "v{version}");
    }
}

#[test]
fn a_redefinition_that_calls_a_sleeping_original_sleeps() {
    let code = r##"
/datum/proc/loud()
    sleep(1)
/datum/loud()
    ..()
"##
    .trim();
    let allowlist = sleep_allowlist_for_test(code, 3);
    assert!(!admitted(&allowlist, "/datum/proc/loud"));
    assert!(!admitted(&allowlist, "/datum/loud"));
}

// two overrides on one type share a spelling in the .dmb, so one listing
// covers both
#[test]
fn copies_that_share_a_spelling_are_admitted_only_if_every_copy_is() {
    let code = r##"
/datum/proc/act()
/datum/child/act()
/datum/child/act()
    sleep(1)
"##
    .trim();
    assert!(!admitted(
        &sleep_allowlist_for_test(code, 3),
        "/datum/child/act"
    ));
}
