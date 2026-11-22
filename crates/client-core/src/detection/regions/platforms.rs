// SPDX-License-Identifier: AGPL-3.0-only
// Native ports of RegionRestrictionCheck decisions; maintained directly in Rust.
// Upstream revision and modification notes: resources/ip-check/SOURCE.md.
// No shell interpreter or external utility is used at runtime.
use super::*;

pub(super) async fn execute(id: &str, ctx: &mut Context) -> Result<Option<String>> {
    let output = match id {
        "MediaUnlockTest_DisneyPlus"
        | "MediaUnlockTest_ESPNPlus"
        | "MediaUnlockTest_NeonTV"
        | "MediaUnlockTest_HBOMax"
        | "MediaUnlockTest_NLZIET"
        | "MediaUnlockTest_EurosportRO"
        | "MediaUnlockTest_OptusSports"
        | "MediaUnlockTest_HuluUS"
        | "MediaUnlockTest_Catchplay"
        | "MediaUnlockTest_TrueID"
        | "MediaUnlockTest_EPIX"
        | "MediaUnlockTest_Joyn"
        | "MediaUnlockTest_SetantaSports"
        | "MediaUnlockTest_TVer"
        | "MediaUnlockTest_Tving"
        | "MediaUnlockTest_Wavve"
        | "MediaUnlockTest_DMM"
        | "MediaUnlockTest_encoreTVB"
        | "MediaUnlockTest_SKY_DE"
        | "MediaUnlockTest_MaoriTV"
        | "MediaUnlockTest_SpotvNow"
        | "MediaUnlockTest_NetflixCDN"
        | "MediaUnlockTest_7plus"
        | "MediaUnlockTest_NaverTV"
        | "MediaUnlockTest_TV360"
        | "MediaUnlockTest_KPlus"
        | "MediaUnlockTest_ofiii"
        | "MediaUnlockTest_NowE"
        | "MediaUnlockTest_ViuTV"
        | "MediaUnlockTest_4GTV"
        | "MediaUnlockTest_Channel5" => return Ok(None),
        r#"MediaUnlockTest_ChatGPT"# => mediaunlocktest_chatgpt(ctx).await,
        r#"AIUnlockTest_Claude"# => aiunlocktest_claude(ctx).await,
        r#"AIUnlockTest_Gemini_location"# => aiunlocktest_gemini_location(ctx).await,
        r#"AIUnlockTest_Copilot"# => aiunlocktest_copilot(ctx).await,
        r#"MediaUnlockTest_PrimeVideo_Region"# => mediaunlocktest_primevideo_region(ctx).await,
        r#"MediaUnlockTest_BilibiliAnimeNew"# => mediaunlocktest_bilibilianimenew(ctx).await,
        r#"MediaUnlockTest_Dazn"# => mediaunlocktest_dazn(ctx).await,
        r#"MediaUnlockTest_Google"# => mediaunlocktest_google(ctx).await,
        r#"MediaUnlockTest_HotStar"# => mediaunlocktest_hotstar(ctx).await,
        r#"MediaUnlockTest_Instagram.Music"# => mediaunlocktest_instagram_music(ctx).await,
        r#"MediaUnlockTest_iQYI_Region"# => mediaunlocktest_iqyi_region(ctx).await,
        r#"MediaUnlockTest_Netflix"# => mediaunlocktest_netflix(ctx).await,
        r#"MediaUnlockTest_Spotify"# => mediaunlocktest_spotify(ctx).await,
        r#"GameTest_Steam"# => gametest_steam(ctx).await,
        r#"MediaUnlockTest_Tiktok"# => mediaunlocktest_tiktok(ctx).await,
        r#"MediaUnlockTest_TVBAnywhere"# => mediaunlocktest_tvbanywhere(ctx).await,
        r#"MediaUnlockTest_Viu.com"# => mediaunlocktest_viu_com(ctx).await,
        r#"MediaUnlockTest_Wikipedia_Editable"# => mediaunlocktest_wikipedia_editable(ctx).await,
        r#"MediaUnlockTest_YouTube_Premium"# => mediaunlocktest_youtube_premium(ctx).await,
        r#"MediaUnlockTest_YouTube_CDN"# => mediaunlocktest_youtube_cdn(ctx).await,
        r#"MediaUnlockTest_BahamutAnime"# => mediaunlocktest_bahamutanime(ctx).await,
        r#"MediaUnlockTest_BilibiliTW"# => mediaunlocktest_bilibilitw(ctx).await,
        r#"MediaUnlockTest_FridayVideo"# => mediaunlocktest_fridayvideo(ctx).await,
        r#"MediaUnlockTest_HamiVideo"# => mediaunlocktest_hamivideo(ctx).await,
        r#"MediaUnlockTest_HBOGO_ASIA"# => mediaunlocktest_hbogo_asia(ctx).await,
        r#"MediaUnlockTest_KKTV"# => mediaunlocktest_kktv(ctx).await,
        r#"MediaUnlockTest_LineTV.TW"# => mediaunlocktest_linetv_tw(ctx).await,
        r#"MediaUnlockTest_LiTV"# => mediaunlocktest_litv(ctx).await,
        r#"MediaUnlockTest_MyVideo"# => mediaunlocktest_myvideo(ctx).await,
        r#"MediaUnlockTest_BilibiliHKMCTW"# => mediaunlocktest_bilibilihkmctw(ctx).await,
        r#"MediaUnlockTest_HoyTV"# => mediaunlocktest_hoytv(ctx).await,
        r#"MediaUnlockTest_MyTVSuper"# => mediaunlocktest_mytvsuper(ctx).await,
        r#"MediaUnlockTest_NBATV"# => mediaunlocktest_nbatv(ctx).await,
        r#"MediaUnlockTest_AbemaTV_IPTest"# => mediaunlocktest_abematv_iptest(ctx).await,
        r#"MediaUnlockTest_AnimeFesta"# => mediaunlocktest_animefesta(ctx).await,
        r#"MediaUnlockTest_DAnimeStore"# => mediaunlocktest_danimestore(ctx).await,
        r#"MediaUnlockTest_DMMTV"# => mediaunlocktest_dmmtv(ctx).await,
        r#"MediaUnlockTest_EroGameSpace"# => mediaunlocktest_erogamespace(ctx).await,
        r#"MediaUnlockTest_FOD"# => mediaunlocktest_fod(ctx).await,
        r#"MediaUnlockTest_HuluJP"# => mediaunlocktest_hulujp(ctx).await,
        r#"MediaUnlockTest_J:COM_ON_DEMAND"# => mediaunlocktest_j_com_on_demand(ctx).await,
        r#"MediaUnlockTest_Kancolle"# => mediaunlocktest_kancolle(ctx).await,
        r#"MediaUnlockTest_DAM"# => mediaunlocktest_dam(ctx).await,
        r#"MediaUnlockTest_KonosubaFD"# => mediaunlocktest_konosubafd(ctx).await,
        r#"MediaUnlockTest_Lemino"# => mediaunlocktest_lemino(ctx).await,
        r#"MediaUnlockTest_MGStage"# => mediaunlocktest_mgstage(ctx).await,
        r#"MediaUnlockTest_mora"# => mediaunlocktest_mora(ctx).await,
        r#"MediaUnlockTest_music.jp"# => mediaunlocktest_music_jp(ctx).await,
        r#"MediaUnlockTest_NHKPlus"# => mediaunlocktest_nhkplus(ctx).await,
        r#"MediaUnlockTest_Niconico"# => mediaunlocktest_niconico(ctx).await,
        r#"MediaUnlockTest_Paravi"# => mediaunlocktest_paravi(ctx).await,
        r#"MediaUnlockTest_UMAJP"# => mediaunlocktest_umajp(ctx).await,
        r#"MediaUnlockTest_PCRJP"# => mediaunlocktest_pcrjp(ctx).await,
        r#"MediaUnlockTest_ProjectSekai"# => mediaunlocktest_projectsekai(ctx).await,
        r#"MediaUnlockTest_Radiko"# => mediaunlocktest_radiko(ctx).await,
        r#"MediaUnlockTest_RakutenMagazine"# => mediaunlocktest_rakutenmagazine(ctx).await,
        r#"MediaUnlockTest_RakutenTVJP"# => mediaunlocktest_rakutentvjp(ctx).await,
        r#"MediaUnlockTest_Telasa"# => mediaunlocktest_telasa(ctx).await,
        r#"MediaUnlockTest_unext"# => mediaunlocktest_unext(ctx).await,
        r#"MediaUnlockTest_VideoMarket"# => mediaunlocktest_videomarket(ctx).await,
        r#"MediaUnlockTest_wowow"# => mediaunlocktest_wowow(ctx).await,
        r#"MediaUnlockTest_AETV"# => mediaunlocktest_aetv(ctx).await,
        r#"MediaUnlockTest_AcornTV"# => mediaunlocktest_acorntv(ctx).await,
        r#"MediaUnlockTest_BritBox"# => mediaunlocktest_britbox(ctx).await,
        r#"MediaUnlockTest_CBCGem"# => mediaunlocktest_cbcgem(ctx).await,
        r#"MediaUnlockTest_Crackle"# => mediaunlocktest_crackle(ctx).await,
        r#"MediaUnlockTest_Crave"# => mediaunlocktest_crave(ctx).await,
        r#"MediaUnlockTest_Crunchyroll"# => mediaunlocktest_crunchyroll(ctx).await,
        r#"MediaUnlockTest_CWTV"# => mediaunlocktest_cwtv(ctx).await,
        r#"MediaUnlockTest_ATTNOW"# => mediaunlocktest_attnow(ctx).await,
        r#"MediaUnlockTest_DiscoveryPlus"# => mediaunlocktest_discoveryplus(ctx).await,
        r#"MediaUnlockTest_Fox"# => mediaunlocktest_fox(ctx).await,
        r#"MediaUnlockTest_FuboTV"# => mediaunlocktest_fubotv(ctx).await,
        r#"MediaUnlockTest_FXNOW"# => mediaunlocktest_fxnow(ctx).await,
        r#"MediaUnlockTest_KBSAmerican"# => mediaunlocktest_kbsamerican(ctx).await,
        r#"MediaUnlockTest_KOCOWA"# => mediaunlocktest_kocowa(ctx).await,
        r#"MediaUnlockTest_MathsSpot"# => mediaunlocktest_mathsspot(ctx).await,
        r#"MediaUnlockTest_NBCTV"# => mediaunlocktest_nbctv(ctx).await,
        r#"MediaUnlockTest_NFLPlus"# => mediaunlocktest_nflplus(ctx).await,
        r#"MediaUnlockTest_ParamountPlus"# => mediaunlocktest_paramountplus(ctx).await,
        r#"MediaUnlockTest_PeacockTV"# => mediaunlocktest_peacocktv(ctx).await,
        r#"MediaUnlockTest_Philo"# => mediaunlocktest_philo(ctx).await,
        r#"MediaUnlockTest_PlutoTV"# => mediaunlocktest_plutotv(ctx).await,
        r#"MediaUnlockTest_Popcornflix"# => mediaunlocktest_popcornflix(ctx).await,
        r#"MediaUnlockTest_SlingTV"# => mediaunlocktest_slingtv(ctx).await,
        r#"MediaUnlockTest_SHOWTIME"# => mediaunlocktest_showtime(ctx).await,
        r#"MediaUnlockTest_Shudder"# => mediaunlocktest_shudder(ctx).await,
        r#"MediaUnlockTest_Starz"# => mediaunlocktest_starz(ctx).await,
        r#"MediaUnlockTest_TLCGO"# => mediaunlocktest_tlcgo(ctx).await,
        r#"MediaUnlockTest_TubiTV"# => mediaunlocktest_tubitv(ctx).await,
        r#"MediaUnlockTest_DirecTVGO"# => mediaunlocktest_directvgo(ctx).await,
        r#"MediaUnlockTest_Amediateka"# => mediaunlocktest_amediateka(ctx).await,
        r#"MediaUnlockTest_BBCiPLAYER"# => mediaunlocktest_bbciplayer(ctx).await,
        r#"MediaUnlockTest_CanalPlus"# => mediaunlocktest_canalplus(ctx).await,
        r#"MediaUnlockTest_Channel4"# => mediaunlocktest_channel4(ctx).await,
        r#"MediaUnlockTest_DiscoveryPlusUK"# => mediaunlocktest_discoveryplusuk(ctx).await,
        r#"MediaUnlockTest_Eurosport"# => mediaunlocktest_eurosport(ctx).await,
        r#"MediaUnlockTest_FranceTV"# => mediaunlocktest_francetv(ctx).await,
        r#"MediaUnlockTest_HBO_Nordic"# => mediaunlocktest_hbo_nordic(ctx).await,
        r#"MediaUnlockTest_HBO_Spain"# => mediaunlocktest_hbo_spain(ctx).await,
        r#"MediaUnlockTest_ITVHUB"# => mediaunlocktest_itvhub(ctx).await,
        r#"MediaUnlockTest_MegogoTV"# => mediaunlocktest_megogotv(ctx).await,
        r#"MediaUnlockTest_Molotov"# => mediaunlocktest_molotov(ctx).await,
        r#"MediaUnlockTest_MoviStarPlus"# => mediaunlocktest_movistarplus(ctx).await,
        r#"MediaUnlockTest_NPO_Start_Plus"# => mediaunlocktest_npo_start_plus(ctx).await,
        r#"MediaUnlockTest_RaiPlay"# => mediaunlocktest_raiplay(ctx).await,
        r#"MediaUnlockTest_RakutenTV"# => mediaunlocktest_rakutentv(ctx).await,
        r#"MediaUnlockTest_Salto"# => mediaunlocktest_salto(ctx).await,
        r#"MediaUnlockTest_Sky_CH"# => mediaunlocktest_sky_ch(ctx).await,
        r#"MediaUnlockTest_SkyGo"# => mediaunlocktest_skygo(ctx).await,
        r#"MediaUnlockTest_SkyShowTime"# => mediaunlocktest_skyshowtime(ctx).await,
        r#"MediaUnlockTest_Viaplay"# => mediaunlocktest_viaplay(ctx).await,
        r#"MediaUnlockTest_videoland"# => mediaunlocktest_videoland(ctx).await,
        r#"MediaUnlockTest_ZDF"# => mediaunlocktest_zdf(ctx).await,
        r#"MediaUnlockTest_ABCiView"# => mediaunlocktest_abciview(ctx).await,
        r#"MediaUnlockTest_Binge"# => mediaunlocktest_binge(ctx).await,
        r#"MediaUnlockTest_Channel10"# => mediaunlocktest_channel10(ctx).await,
        r#"MediaUnlockTest_Channel9"# => mediaunlocktest_channel9(ctx).await,
        r#"MediaUnlockTest_Docplay"# => mediaunlocktest_docplay(ctx).await,
        r#"MediaUnlockTest_KayoSports"# => mediaunlocktest_kayosports(ctx).await,
        r#"MediaUnlockTest_SBSonDemand"# => mediaunlocktest_sbsondemand(ctx).await,
        r#"MediaUnlockTest_SkyGONZ"# => mediaunlocktest_skygonz(ctx).await,
        r#"MediaUnlockTest_Stan"# => mediaunlocktest_stan(ctx).await,
        r#"MediaUnlockTest_ThreeNow"# => mediaunlocktest_threenow(ctx).await,
        r#"MediaUnlockTest_Afreeca"# => mediaunlocktest_afreeca(ctx).await,
        r#"MediaUnlockTest_CoupangPlay"# => mediaunlocktest_coupangplay(ctx).await,
        r#"MediaUnlockTest_KBSDomestic"# => mediaunlocktest_kbsdomestic(ctx).await,
        r#"MediaUnlockTest_PandaTV"# => mediaunlocktest_pandatv(ctx).await,
        r#"MediaUnlockTest_AISPlay"# => mediaunlocktest_aisplay(ctx).await,
        r#"MediaUnlockTest_beIN_Sports"# => mediaunlocktest_bein_sports(ctx).await,
        r#"MediaUnlockTest_ClipTV"# => mediaunlocktest_cliptv(ctx).await,
        r#"MediaUnlockTest_GalaxyPlay"# => mediaunlocktest_galaxyplay(ctx).await,
        r#"MediaUnlockTest_JioCinema"# => mediaunlocktest_jiocinema(ctx).await,
        r#"MediaUnlockTest_meWATCH"# => mediaunlocktest_mewatch(ctx).await,
        r#"MediaUnlockTest_MXPlayer"# => mediaunlocktest_mxplayer(ctx).await,
        r#"MediaUnlockTest_MYTV"# => mediaunlocktest_mytv(ctx).await,
        r#"MediaUnlockTest_SonyLiv"# => mediaunlocktest_sonyliv(ctx).await,
        r#"MediaUnlockTest_Sooka"# => mediaunlocktest_sooka(ctx).await,
        r#"MediaUnlockTest_StarhubTVPlus"# => mediaunlocktest_starhubtvplus(ctx).await,
        r#"MediaUnlockTest_TataPlay"# => mediaunlocktest_tataplay(ctx).await,
        r#"MediaUnlockTest_Vidio"# => mediaunlocktest_vidio(ctx).await,
        r#"MediaUnlockTest_VTVcab"# => mediaunlocktest_vtvcab(ctx).await,
        r#"MediaUnlockTest_Zee5"# => mediaunlocktest_zee5(ctx).await,
        r#"MediaUnlockTest_DStv"# => mediaunlocktest_dstv(ctx).await,
        r#"MediaUnlockTest_BeinConnect"# => mediaunlocktest_beinconnect(ctx).await,
        r#"MediaUnlockTest_MolaTV"# => mediaunlocktest_molatv(ctx).await,
        r#"MediaUnlockTest_CineMax"# => mediaunlocktest_cinemax(ctx).await,
        r#"MediaUnlockTest_HBONow"# => mediaunlocktest_hbonow(ctx).await,
        r#"MediaUnlockTest_HBO_Portugal"# => mediaunlocktest_hbo_portugal(ctx).await,
        _ => bail!("检测项目不存在"),
    }?;
    Ok(Some(output))
}

async fn mediaunlocktest_chatgpt(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://chatgpt.com"#.to_owned(),
            method: r#"HEAD"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: true,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    v.set(
        r#"tmpresult1"#,
        ctx.request(Request {
            url: r#"https://ios.chat.openai.com"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    v.set(
        r#"cf_details"#,
        json_field(
            &[v.get(r#"tmpresult1"#)].join(" "),
            &r#".cf_details"#.to_owned(),
        )?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r ChatGPT:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result1"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"-i"#.to_owned(), r#"^location:"#.to_owned()],
        )?,
    );
    if (!(!(v.get(r#"result1"#)).is_empty())) {
        if (matches_pattern(
            &v.get(r#"tmpresult1"#),
            &r#"*blocked_why_headline*"#.to_owned(),
        )) {
            output = decode_escapes(
                &[[
                    r#"\r ChatGPT:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Red"#),
                    r#"No (Blocked)"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        }
        if (matches_pattern(
            &v.get(r#"tmpresult1"#),
            &r#"*unsupported_country_region_territory*"#.to_owned(),
        )) {
            output = decode_escapes(
                &[[
                    r#"\r ChatGPT:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Red"#),
                    r#"No (Unsupported Region)"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        }
        if (matches_pattern(&v.get(r#"cf_details"#), &r#"*(1)*"#.to_owned())) {
            output = decode_escapes(
                &[[
                    r#"\r ChatGPT:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Red"#),
                    r#"No (Disallowed ISP[1])"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        }
        if (matches_pattern(&v.get(r#"cf_details"#), &r#"*(2)*"#.to_owned())) {
            output = decode_escapes(
                &[[
                    r#"\r ChatGPT:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Red"#),
                    r#"No (Disallowed ISP[2])"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        }
        v.set(
            r#"main_status"#,
            select_fields(
                &[v.get(r#"tmpresult"#)].join(" "),
                &[r#"/^HTTP\// { code=$2 } END { print code }"#.to_owned()],
            )?,
        );
        v.set(
            r#"trace"#,
            ctx.request(Request {
                url: r#"https://chatgpt.com/cdn-cgi/trace"#.to_owned(),
                method: r#"GET"#.to_owned(),
                headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
                body: None,
                follow: true,
                headers_only: false,
                include_headers: false,
                fail_status: false,
                discard_body: false,
                timeout: 10,
                writeout: String::new(),
                cookie_store: false,
                cookie_load: false,
                retries: 0,
                tls13: false,
            })
            .await?,
        );
        v.set(r#"region"#, String::new());
        if !(select_lines(
            &[v.get(r#"trace"#)].join(" "),
            &[r#"-q"#.to_owned(), r#"^h=chatgpt.com"#.to_owned()],
        )?)
        .is_empty()
        {
            v.set(
                r#"region"#,
                map_characters(
                    &split_fields(
                        &select_lines(
                            &[v.get(r#"trace"#)].join(" "),
                            &[r#"-E"#.to_owned(), r#"^loc=[A-Z]{2}"#.to_owned()],
                        )?,
                        &[r#"-d="#.to_owned(), r#"-f2"#.to_owned()],
                    )?,
                    &[r#"-d"#.to_owned(), r#"\r"#.to_owned()],
                )?,
            );
        }
        if (v.get(r#"main_status"#) == r#"200"#.to_owned()) {
            output = decode_escapes(
                &[[
                    r#"\r ChatGPT:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Green"#),
                    r#"Web Reachable (Region: "#.to_owned(),
                    v.get(r#"region"#),
                    r#")"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
        } else {
            output = decode_escapes(
                &[[
                    r#"\r ChatGPT:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Yellow"#),
                    r#"No (Region: "#.to_owned(),
                    v.get(r#"region"#),
                    r#")"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
        }
    } else {
        v.set(
            r#"region1"#,
            select_fields(
                &select_lines(
                    &ctx.request(Request {
                        url: r#"https://chatgpt.com/cdn-cgi/trace"#.to_owned(),
                        method: r#"GET"#.to_owned(),
                        headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
                        body: None,
                        follow: true,
                        headers_only: false,
                        include_headers: false,
                        fail_status: false,
                        discard_body: false,
                        timeout: 10,
                        writeout: String::new(),
                        cookie_store: false,
                        cookie_load: false,
                        retries: 0,
                        tls13: false,
                    })
                    .await?,
                    &[r#"loc="#.to_owned()],
                )?,
                &[r#"-F="#.to_owned(), r#"{print $2}"#.to_owned()],
            )?,
        );
        if (matches_pattern(&v.get(r#"cf_details"#), &r#"*(1)*"#.to_owned())) {
            output = decode_escapes(
                &[[
                    r#"\r ChatGPT:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Yellow"#),
                    r#"Web Only (Disallowed ISP[1])"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        }
        if (matches_pattern(&v.get(r#"cf_details"#), &r#"*(2)*"#.to_owned())) {
            output = decode_escapes(
                &[[
                    r#"\r ChatGPT:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Yellow"#),
                    r#"Web Only (Disallowed ISP[2])"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        }
        output = decode_escapes(
            &[[
                r#"\r ChatGPT:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes (Region: "#.to_owned(),
                v.get(r#"region1"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn aiunlocktest_claude(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        select_lines(
            &ctx.request(Request {
                url: r#"https://claude.ai/"#.to_owned(),
                method: r#"GET"#.to_owned(),
                headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
                body: None,
                follow: true,
                headers_only: false,
                include_headers: false,
                fail_status: false,
                discard_body: true,
                timeout: 10,
                writeout: r#"%{url_effective}%{http_code}\n"#.to_owned(),
                cookie_store: false,
                cookie_load: false,
                retries: 0,
                tls13: false,
            })
            .await?,
            &[r#"-E"#.to_owned(), r#"unavailable|000"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Claude:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Claude:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Claude:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed (Network Connection)"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

async fn aiunlocktest_gemini_location(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmp"#,
        ctx.request(Request {
            url: r#"https://gemini.google.com/_/BardChatUi/data/batchexecute"#.to_owned(),
            method: r#"POST"#.to_owned(),
            headers: vec![
                [r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)],
                header_pair(&r#"accept-language: en-US"#.to_owned())?,
            ],
            body: Some(
                [r#"f.req=[[["K4WWud","[[0],[\"en-US\"]]",null,"generic"]]]"#.to_owned()].join("&"),
            ),
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmp"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Google Gemini Location:\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"region"#,
        select_lines(
            &json_field(
                &select_lines(&[v.get(r#"tmp"#)].join(" "), &[r#"K4WWud"#.to_owned()])?,
                &r#".[0][2]"#.to_owned(),
            )?,
            &[r#"-Eo"#.to_owned(), r#"\[\[\\"(.*)\\",\\"S"#.to_owned()],
        )?,
    );
    output = decode_escapes(
        &[[
            r#"\r Google Gemini Location:\t\t"#.to_owned(),
            v.get(r#"Font_Yellow"#),
            slice_text(&v.get(r#"region"#), 4, Some(-6)),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn aiunlocktest_copilot(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmp"#,
        ctx.request(Request {
            url: r#"https://copilot.microsoft.com/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    v.set(r#"tmp2"#, ctx.request(Request { url: r#"https://copilot.microsoft.com/turing/conversation/chats?bundleVersion=1.1342.3-cplt.12"#.to_owned(), method: r#"GET"#.to_owned(), headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]], body: None, follow: true, headers_only: false, include_headers: false, fail_status: false, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (matches_pattern(&v.get(r#"tmp"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Microsoft Copilot:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        map_characters(
            &json_field(
                &[v.get(r#"tmp2"#)].join(" "),
                &r#".result.value"#.to_owned(),
            )?,
            &[r#"-d"#.to_owned(), r#"""#.to_owned()],
        )?,
    );
    v.set(
        r#"region"#,
        replace_text(
            &[v.get(r#"tmp"#)].join(" "),
            &[
                r#"-n"#.to_owned(),
                r#"s/.*RevIpCC:"\([^"]*\)".*/\1/p"#.to_owned(),
            ],
        )?,
    );
    if (v.get(r#"result"#) == r#"Success"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Microsoft Copilot:\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes (Region: "#.to_owned(),
                v.get(r#"region"#).to_uppercase(),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r Microsoft Copilot:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No  (Region: "#.to_owned(),
                v.get(r#"region"#).to_uppercase(),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_primevideo_region(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://www.primevideo.com"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Amazon Prime Video:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(r#"tmpresult1"#, ctx.request(Request { url: format!(r#"https://ab9f7h23rcdn.eu.api.amazonvideo.com/cdp/appleedge/getDataByTransform/v1/apple/detail/vod/v1.kt?itemId=amzn1.dv.gti.e6b39984-2bb6-f7d0-33e4-08ec574947f0&deviceId={}&deviceTypeId=A3JTVZS31ZJ340&density=2x&firmware=10.6800.16104.3&format=json&enabledFeatures=denarius.location.gen4.daric.siglos.siglosPartnerBilling.contentDescriptors.contentDescriptorsV2.productPlacement.zeno.seriesSearch.tapsV2.dateTimeLocalization.multiSourcedEvents.mseEventLevelOffers.liveWatchModal.lbv.daapi.maturityRatingDecoration.seasonTrailer.cleanSlate.xbdModalV2.xbdModalVdp.playbackPinV2.exploreTab.reactions.progBadging.atfEpTimeVis.prereleaseCx.vppaConsent.episodicRelease.movieVam.movieVamCatalog&journeyIngressContext=8%7CEgRzdm9k&osLocale=zh_Hans_CN&timeZoneId=Asia%2FShanghai&uxLocale=zh_CN"#, random_id()), method: r#"GET"#.to_owned(), headers: vec![[r#"user-agent"#.to_owned(), r#"PrimeVideo/10.68 (iPad; iOS 18.3.2; Scale/2.00)"#.to_owned()]], body: None, follow: true, headers_only: false, include_headers: false, fail_status: false, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (matches_pattern(&v.get(r#"tmpresult1"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Amazon Prime Video:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"VPNDetected"#,
        select_lines(
            &[v.get(r#"tmpresult1"#)].join(" "),
            &[r#"您的设备使用了 VPN 或代理服务连接互联网请禁用并重试"#.to_owned()],
        )?,
    );
    v.set(
        r#"result"#,
        take_lines(
            &split_fields(
                &replace_text(
                    &select_lines(
                        &[v.get(r#"tmpresult"#)].join(" "),
                        &[r#""currentTerritory":"#.to_owned()],
                    )?,
                    &[r#"s/.*currentTerritory//"#.to_owned()],
                )?,
                &[r#"-f3"#.to_owned(), r#"-d""#.to_owned()],
            )?,
            &[r#"-n"#.to_owned(), r#"1"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        if (!(v.get(r#"VPNDetected"#)).is_empty()) {
            output = decode_escapes(
                &[[
                    r#"\r Amazon Prime Video:\t\t\t"#.to_owned(),
                    v.get(r#"Font_Red"#),
                    r#"No  (VPN Detected;Region: "#.to_owned(),
                    v.get(r#"result"#),
                    r#")"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        } else {
            output = decode_escapes(
                &[[
                    r#"\r Amazon Prime Video:\t\t\t"#.to_owned(),
                    v.get(r#"Font_Green"#),
                    r#"Yes (Region: "#.to_owned(),
                    v.get(r#"result"#),
                    r#")"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        }
    } else {
        output = decode_escapes(
            &[[
                r#"\r Amazon Prime Video:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Unsupported"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_bilibilianimenew(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    if (v.get(r#"1"#) == r#"6"#.to_owned()) {
        v.set(
            r#"bili_ip6"#,
            select_fields(
                &take_lines(
                    &ctx.resolve_host(&r#"api.bilibili.com"#.to_owned(), &r#"ahostsv6"#.to_owned())
                        .await?,
                    &[r#"-1"#.to_owned()],
                )?,
                &[r#"{print $1}"#.to_owned()],
            )?,
        );
        if ((v.get(r#"bili_ip6"#)).is_empty()) {
            v.set(r#"bili_ip6"#, r#"2409:8c54:1841:2002::22"#.to_owned());
            return Ok(output);
        }
        v.set(
            r#"tmp"#,
            ctx.request(Request {
                url: r#"https://api.bilibili.com/x/web-interface/zone"#.to_owned(),
                method: r#"GET"#.to_owned(),
                headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
                body: None,
                follow: true,
                headers_only: false,
                include_headers: false,
                fail_status: true,
                discard_body: false,
                timeout: 10,
                writeout: String::new(),
                cookie_store: false,
                cookie_load: false,
                retries: 0,
                tls13: false,
            })
            .await?,
        );
    } else {
        v.set(
            r#"tmp"#,
            ctx.request(Request {
                url: r#"https://api.bilibili.com/x/web-interface/zone"#.to_owned(),
                method: r#"GET"#.to_owned(),
                headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
                body: None,
                follow: true,
                headers_only: false,
                include_headers: false,
                fail_status: true,
                discard_body: false,
                timeout: 10,
                writeout: String::new(),
                cookie_store: false,
                cookie_load: false,
                retries: 0,
                tls13: false,
            })
            .await?,
        );
    }
    if (matches_pattern(&v.get(r#"tmp"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Bilibili Anime:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"country_code"#,
        json_field(
            &[v.get(r#"tmp"#)].join(" "),
            &r#".data.country_code"#.to_owned(),
        )?,
    );
    if (v.get(r#"country_code"#) == r#"86"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Bilibili Anime:\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes (Region: CN)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"country_code"#) == r#"886"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Bilibili Anime:\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes (Region: TW)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"country_code"#) == r#"852"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Bilibili Anime:\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes (Region: HK)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"country_code"#) == r#"853"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Bilibili Anime:\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes (Region: MO)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        v.set(
            r#"country"#,
            map_characters(
                &json_field(&[v.get(r#"tmp"#)].join(" "), &r#".data.country"#.to_owned())?,
                &[r#"-d"#.to_owned(), r#"""#.to_owned()],
            )?,
        );
        output = decode_escapes(
            &[[
                r#"\r Bilibili Anime:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No  (Country: "#.to_owned(),
                v.get(r#"country"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_dazn(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"tmpresult"#, ctx.request(Request { url: r#"https://startup.core.indazn.com/misl/v5/Startup"#.to_owned(), method: r#"POST"#.to_owned(), headers: vec![header_pair(&r#"Content-Type: application/json"#.to_owned())?], body: Some([r#"{"LandingPageKey":"generic","Languages":"zh-CN,zh,en","Platform":"web","PlatformAttributes":{},"Manufacturer":"","PromoCode":"","Version":"2"}"#.to_owned()].join("&")), follow: false, headers_only: false, include_headers: false, fail_status: false, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Dazn:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (matches_pattern(
        &v.get(r#"tmpresult"#),
        &r#"*Security policy has been breached*"#.to_owned(),
    )) {
        output = decode_escapes(
            &[[
                r#"\r Dazn:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No  (Banned)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"*Forbidden*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Dazn:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No  (Banned)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"isAllowed"#,
        json_field(
            &[v.get(r#"tmpresult"#)].join(" "),
            &r#".Region.isAllowed"#.to_owned(),
        )?,
    );
    v.set(
        r#"region"#,
        map_characters(
            &json_field(
                &[v.get(r#"tmpresult"#)].join(" "),
                &r#".Region.GeolocatedCountry"#.to_owned(),
            )?,
            &[r#"-d"#.to_owned(), r#"""#.to_owned()],
        )?,
    );
    if (v.get(r#"isAllowed"#) == r#"true"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Dazn:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes (Region: "#.to_owned(),
                v.get(r#"region"#).to_uppercase(),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"isAllowed"#) == r#"false"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Dazn:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Dazn:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Unsupport"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_google(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmp"#,
        ctx.request(Request {
            url: r#"https://bard.google.com/_/BardChatUi/data/batchexecute"#.to_owned(),
            method: r#"POST"#.to_owned(),
            headers: vec![
                [r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)],
                header_pair(&r#"accept-language: en-US"#.to_owned())?,
            ],
            body: Some(
                [r#"f.req=[[["K4WWud","[[0],[\"en-US\"]]",null,"generic"]]]"#.to_owned()].join("&"),
            ),
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmp"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Google Location:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"region"#,
        select_lines(
            &json_field(
                &select_lines(&[v.get(r#"tmp"#)].join(" "), &[r#"K4WWud"#.to_owned()])?,
                &r#".[0][2]"#.to_owned(),
            )?,
            &[r#"-Eo"#.to_owned(), r#"\[\[\\"(.*)\\",\\"S"#.to_owned()],
        )?,
    );
    output = decode_escapes(
        &[[
            r#"\r Google Location:\t\t\t"#.to_owned(),
            v.get(r#"Font_Green"#),
            slice_text(&v.get(r#"region"#), 4, Some(-6)),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_hotstar(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://api.hotstar.com/o/v1/page/1557?offset=0&size=20&tao=0&tas=20"#
                .to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r HotStar:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"401"#.to_owned()) {
        v.set(
            r#"region"#,
            split_fields(
                &replace_text(
                    &select_lines(
                        &ctx.request(Request {
                            url: r#"https://www.hotstar.com"#.to_owned(),
                            method: r#"HEAD"#.to_owned(),
                            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
                            body: None,
                            follow: false,
                            headers_only: true,
                            include_headers: false,
                            fail_status: false,
                            discard_body: false,
                            timeout: 10,
                            writeout: String::new(),
                            cookie_store: false,
                            cookie_load: false,
                            retries: 0,
                            tls13: false,
                        })
                        .await?,
                        &[r#"geo="#.to_owned()],
                    )?,
                    &[r#"s/.*geo=//"#.to_owned()],
                )?,
                &[r#"-f1"#.to_owned(), r#"-d,"#.to_owned()],
            )?,
        );
        v.set(
            r#"site_region"#,
            map_characters(
                &replace_text(
                    &ctx.request(Request {
                        url: r#"https://www.hotstar.com"#.to_owned(),
                        method: r#"GET"#.to_owned(),
                        headers: vec![],
                        body: None,
                        follow: true,
                        headers_only: false,
                        include_headers: false,
                        fail_status: false,
                        discard_body: true,
                        timeout: 10,
                        writeout: r#"%{url_effective}\n"#.to_owned(),
                        cookie_store: false,
                        cookie_load: false,
                        retries: 0,
                        tls13: false,
                    })
                    .await?,
                    &[r#"s@.*com/@@"#.to_owned()],
                )?,
                &[r#"[:lower:]"#.to_owned(), r#"[:upper:]"#.to_owned()],
            )?,
        );
        if (!(v.get(r#"region"#)).is_empty()) && (v.get(r#"region"#) == v.get(r#"site_region"#)) {
            output = decode_escapes(
                &[[
                    r#"\r HotStar:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Green"#),
                    r#"Yes (Region: "#.to_owned(),
                    v.get(r#"region"#),
                    r#")"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        } else {
            output = decode_escapes(
                &[[
                    r#"\r HotStar:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Red"#),
                    r#"No"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        }
    } else if (v.get(r#"result"#) == r#"475"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r HotStar:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r HotStar:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_instagram_music(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"tmpresult"#, ctx.request(Request { url: r#"https://www.instagram.com/graphql/query"#.to_owned(), method: r#"POST"#.to_owned(), headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]], body: Some([r#"fb_api_caller_class=RelayModern&fb_api_req_friendly_name=PolarisPostActionLoadPostQueryQuery&variables=%7B%22shortcode%22%3A%22C2YEAdOh9AB%22%2C%22fetch_comment_count%22%3A0%2C%22parent_comment_count%22%3A0%2C%22child_comment_count%22%3A0%2C%22fetch_like_count%22%3A0%2C%22fetch_tagged_user_count%22%3Anull%2C%22fetch_preview_comment_count%22%3A0%2C%22has_threaded_comments%22%3Atrue%2C%22hoisted_comment_id%22%3Anull%2C%22hoisted_reply_id%22%3Anull%7D&server_timestamps=true&doc_id=25531498899829322"#.to_owned()].join("&")), follow: false, headers_only: false, include_headers: false, fail_status: false, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Instagram Licensed Music:\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        json_field(
            &[v.get(r#"tmpresult"#)].join(" "),
            &r#".data.xdt_shortcode_media.clips_music_attribution_info.should_mute_audio"#
                .to_owned(),
        )?,
    );
    if (v.get(r#"result"#) == r#"false"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Instagram Licensed Music:\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"true"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Instagram Licensed Music:\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r Instagram Licensed Music:\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Unexpected Result)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_iqyi_region(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://www.iq.com/"#.to_owned(),
            method: r#"HEAD"#.to_owned(),
            headers: vec![],
            body: None,
            follow: false,
            headers_only: true,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r iQyi Oversea:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if ((v.get(r#"tmpresult"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r iQyi Oversea:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        split_fields(
            &split_fields(
                &select_fields(
                    &select_lines(&[v.get(r#"tmpresult"#)].join(" "), &[r#"mod="#.to_owned()])?,
                    &[r#"{print $2}"#.to_owned()],
                )?,
                &[r#"-f2"#.to_owned(), r#"-d="#.to_owned()],
            )?,
            &[r#"-f1"#.to_owned(), r#"-d;"#.to_owned()],
        )?,
    );
    v.set(
        r#"region"#,
        replace_text(
            &split_fields(
                &select_lines(
                    &[v.get(r#"tmpresult"#)].join(" "),
                    &[r#"x-custom-client-ip:"#.to_owned()],
                )?,
                &[r#"-f3"#.to_owned(), r#"-d:"#.to_owned()],
            )?,
            &[r#"s/.$//"#.to_owned()],
        )?,
    );
    if (v.get(r#"region"#) == r#"cn"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r iQyi Oversea:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Yellow"#),
                r#"Mainland (Region: "#.to_owned(),
                v.get(r#"region"#).to_uppercase(),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (!(v.get(r#"result"#)).is_empty()) {
        if (v.get(r#"result"#) == r#"intl"#.to_owned()) {
            output = decode_escapes(
                &[[
                    r#"\r iQyi Oversea:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Red"#),
                    r#"No  (Region: "#.to_owned(),
                    v.get(r#"region"#).to_uppercase(),
                    r#")"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        } else {
            output = decode_escapes(
                &[[
                    r#"\r iQyi Oversea:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Green"#),
                    r#"Yes (Region: "#.to_owned(),
                    v.get(r#"region"#).to_uppercase(),
                    r#")"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        }
    } else {
        output = decode_escapes(
            &[[
                r#"\r iQyi Oversea:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_netflix(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult1"#,
        ctx.request(Request {
            url: r#"https://www.netflix.com/title/81280792"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: true,
        })
        .await?,
    );
    v.set(
        r#"tmpresult2"#,
        ctx.request(Request {
            url: r#"https://www.netflix.com/title/70143836"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: true,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult1"#), &r#"curl*"#.to_owned()))
        || (matches_pattern(&v.get(r#"tmpresult2"#), &r#"curl*"#.to_owned()))
    {
        output = decode_escapes(
            &[[
                r#"\r Netflix:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result1"#,
        select_lines(
            &[v.get(r#"tmpresult1"#)].join(" "),
            &[r#"og:video"#.to_owned()],
        )?,
    );
    v.set(
        r#"result2"#,
        select_lines(
            &[v.get(r#"tmpresult2"#)].join(" "),
            &[r#"og:video"#.to_owned()],
        )?,
    );
    v.set(
        r#"region1"#,
        map_characters(
            &json_field(
                &replace_text(
                    &map_characters(
                        &decode_escapes(
                            &[select_fields(
                                &select_fields(
                                    &select_lines(
                                        &[v.get(r#"tmpresult1"#)].join(" "),
                                        &[r#"netflix.reactContext"#.to_owned()],
                                    )?,
                                    &[r#"-F="#.to_owned(), r#"{print $2}"#.to_owned()],
                                )?,
                                &[r#"-F;"#.to_owned(), r#"{print $1}"#.to_owned()],
                            )?]
                            .join(" "),
                        ),
                        &[r#"-d"#.to_owned(), r#"[:cntrl:]"#.to_owned()],
                    )?,
                    &[r#"s/\^[^$]*\$//g"#.to_owned()],
                )?,
                &r#".models.geo.data.requestCountry.id"#.to_owned(),
            )?,
            &[r#"-d"#.to_owned(), r#"""#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result1"#)).is_empty()) || (!(v.get(r#"result2"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Netflix:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes (Region: "#.to_owned(),
                v.get(r#"region1"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Netflix:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Yellow"#),
                r#"Originals Only (Region: "#.to_owned(),
                v.get(r#"region1"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Netflix:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_spotify(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://www.spotify.com/tw/signup"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Spotify Region:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"country"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[
                r#"-Eo"#.to_owned(),
                r#"geoCountry.*","geoCountryMarket""#.to_owned(),
            ],
        )?,
    );
    if (!(v.get(r#"country"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Spotify Region:\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                slice_text(&v.get(r#"country"#), 13, Some(-20)),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Spotify Region:\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn gametest_steam(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        split_fields(
            &select_lines(
                &ctx.request(Request {
                    url: r#"https://store.steampowered.com/app/761830"#.to_owned(),
                    method: r#"GET"#.to_owned(),
                    headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
                    body: None,
                    follow: true,
                    headers_only: false,
                    include_headers: false,
                    fail_status: true,
                    discard_body: false,
                    timeout: 10,
                    writeout: String::new(),
                    cookie_store: false,
                    cookie_load: false,
                    retries: 0,
                    tls13: false,
                })
                .await?,
                &[r#"priceCurrency"#.to_owned()],
            )?,
            &[r#"-d"#.to_owned(), r#"""#.to_owned(), r#"-f4"#.to_owned()],
        )?,
    );
    if (!(!(v.get(r#"result"#)).is_empty())) {
        output = decode_escapes(
            &[[
                r#"\r Steam Currency:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r Steam Currency:\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                v.get(r#"result"#),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_tiktok(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://www.tiktok.com/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{url_effective}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    v.set(
        r#"result1"#,
        ctx.request(Request {
            url: r#"https://www.tiktok.com/passport/web/store_region/"#.to_owned(),
            method: r#"POST"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r Tiktok:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Tiktok:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"region"#,
        map_characters(
            &json_field(
                &[v.get(r#"result1"#)].join(" "),
                &r#".data.store_region"#.to_owned(),
            )?,
            &[r#"-d"#.to_owned(), r#"""#.to_owned()],
        )?,
    );
    if (matches_pattern(&v.get(r#"result"#), &r#"*/about"#.to_owned()))
        || (matches_pattern(&v.get(r#"result"#), &r#"*/status*"#.to_owned()))
        || (matches_pattern(&v.get(r#"result"#), &r#"*landing*"#.to_owned()))
    {
        if (v.get(r#"region"#) == r#"cn"#.to_owned()) {
            output = decode_escapes(
                &[[
                    r#"\r Tiktok:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Yellow"#),
                    r#"Provided by Douyin"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        } else {
            output = decode_escapes(
                &[[
                    r#"\r Tiktok:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Red"#),
                    r#"No  (Region: "#.to_owned(),
                    v.get(r#"region"#).to_uppercase(),
                    r#")"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        }
    } else {
        output = decode_escapes(
            &[[
                r#"\r Tiktok:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes (Region: "#.to_owned(),
                v.get(r#"region"#).to_uppercase(),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Tiktok:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_tvbanywhere(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://uapisfm.tvbanywhere.com.sg/geoip/check/platform/android"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if ((v.get(r#"tmpresult"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r TVBAnywhere+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_21Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"region"#,
        map_characters(
            &json_field(
                &[v.get(r#"tmpresult"#)].join(" "),
                &r#".country"#.to_owned(),
            )?,
            &[r#"-d"#.to_owned(), r#"""#.to_owned()],
        )?,
    );
    v.set(
        r#"result"#,
        json_field(
            &[v.get(r#"tmpresult"#)].join(" "),
            &r#".allow_in_this_country"#.to_owned(),
        )?,
    );
    if (v.get(r#"region"#) == r#"HK"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r TVBAnywhere+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Yellow"#),
                r#"Serviced by MyTvSuper (Region: "#.to_owned(),
                v.get(r#"region"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (v.get(r#"result"#) == r#"true"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r TVBAnywhere+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes (Region: "#.to_owned(),
                v.get(r#"region"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"false"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r TVBAnywhere+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No  (Region: "#.to_owned(),
                v.get(r#"region"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r TVBAnywhere+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_viu_com(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://www.viu.com/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{url_effective}\n"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    v.set(
        r#"banresult"#,
        ctx.request(Request {
            url: r#"https://d3o7oi00quuwqu.cloudfront.net"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"tmpresult"#) == r#"000"#.to_owned())
        || (v.get(r#"banresult"#) == r#"curl*"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r Viu.com:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        split_fields(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"-f5"#.to_owned(), r#"-d/"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        if (v.get(r#"result"#) == r#"no-service"#.to_owned()) {
            output = decode_escapes(
                &[[
                    r#"\r Viu.com:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Red"#),
                    r#"No"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        } else {
            v.set(
                r#"result"#,
                map_characters(
                    &[v.get(r#"result"#)].join(" "),
                    &[r#"[:lower:]"#.to_owned(), r#"[:upper:]"#.to_owned()],
                )?,
            );
            if (matches_pattern(&v.get(r#"banresult"#), &r#"*block access*"#.to_owned())) {
                output = decode_escapes(
                    &[[
                        r#"\r Viu.com:\t\t\t\t"#.to_owned(),
                        v.get(r#"Font_Red"#),
                        r#"No  (Region: "#.to_owned(),
                        v.get(r#"result"#),
                        r#")"#.to_owned(),
                        v.get(r#"Font_Suffix"#),
                        r#"\n"#.to_owned(),
                    ]
                    .concat()]
                    .join(" "),
                );
                return Ok(output);
            } else {
                output = decode_escapes(
                    &[[
                        r#"\r Viu.com:\t\t\t\t"#.to_owned(),
                        v.get(r#"Font_Green"#),
                        r#"Yes (Region: "#.to_owned(),
                        v.get(r#"result"#),
                        r#")"#.to_owned(),
                        v.get(r#"Font_Suffix"#),
                        r#"\n"#.to_owned(),
                    ]
                    .concat()]
                    .join(" "),
                );
                return Ok(output);
            }
        }
    } else {
        output = decode_escapes(
            &[[
                r#"\r Viu.com:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_wikipedia_editable(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"tmpresult"#, ctx.request(Request { url: r#"https://en.wikipedia.org/w/index.php?title=Wikipedia:WikiProject_on_open_proxies&action=edit"#.to_owned(), method: r#"GET"#.to_owned(), headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]], body: None, follow: false, headers_only: false, include_headers: false, fail_status: false, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if ((v.get(r#"tmpresult"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Wikipedia Editability:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"-i"#.to_owned(), r#"This IP address has been"#.to_owned()],
        )?,
    );
    if ((v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Wikipedia Editability:\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Wikipedia Editability:\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"No"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_youtube_premium(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://www.youtube.com/premium"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![
                [r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)],
                header_pair(&r#"Accept-Language: en"#.to_owned())?,
            ],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r YouTube Premium:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"isCN"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"www.google.cn"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"isCN"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r YouTube Premium:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#" "#.to_owned(),
                v.get(r#"Font_Green"#),
                r#" (Region: CN)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#" \n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"region"#,
        split_fields(
            &replace_text(
                &select_lines(
                    &[v.get(r#"tmpresult"#)].join(" "),
                    &[r#"countryCode"#.to_owned()],
                )?,
                &[r#"s/.*"countryCode"//"#.to_owned()],
            )?,
            &[r#"-f2"#.to_owned(), r#"-d""#.to_owned()],
        )?,
    );
    v.set(
        r#"isAvailable"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"purchaseButtonOverride"#.to_owned()],
        )?,
    );
    v.set(
        r#"isAvailable2"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"Start trial"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"isAvailable"#)).is_empty())
        || (!(v.get(r#"isAvailable2"#)).is_empty())
        || (!(v.get(r#"region"#)).is_empty())
    {
        if (!(v.get(r#"region"#)).is_empty()) {
            output = decode_escapes(
                &[[
                    r#"\r YouTube Premium:\t\t\t"#.to_owned(),
                    v.get(r#"Font_Green"#),
                    r#"Yes (Region: "#.to_owned(),
                    v.get(r#"region"#),
                    r#")"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        } else {
            output = decode_escapes(
                &[[
                    r#"\r YouTube Premium:\t\t\t"#.to_owned(),
                    v.get(r#"Font_Green"#),
                    r#"Yes"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        }
    } else {
        if (!(v.get(r#"region"#)).is_empty()) {
            output = decode_escapes(
                &[[
                    r#"\r YouTube Premium:\t\t\t"#.to_owned(),
                    v.get(r#"Font_Red"#),
                    r#"No  (Region: "#.to_owned(),
                    v.get(r#"region"#),
                    r#")"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#" \n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        } else {
            output = decode_escapes(
                &[[
                    r#"\r YouTube Premium:\t\t\t"#.to_owned(),
                    v.get(r#"Font_Red"#),
                    r#"Failed (Unexpected Result)"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#" \n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        }
    }
    output = decode_escapes(
        &[[
            r#"\r YouTube Premium:\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_youtube_cdn(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://redirector.googlevideo.com/report_mapping?di=no"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r YouTube Region:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Check Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"cdn_node"#,
        select_fields(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"{print $3}"#.to_owned()],
        )?,
    );
    if (matches_pattern(&v.get(r#"cdn_node"#), &r#"*-*"#.to_owned())) {
        v.set(
            r#"CDN_ISP"#,
            map_characters(
                &split_fields(
                    &[v.get(r#"cdn_node"#)].join(" "),
                    &[r#"-f1"#.to_owned(), r#"-d-"#.to_owned()],
                )?,
                &[r#"[:lower:]"#.to_owned(), r#"[:upper:]"#.to_owned()],
            )?,
        );
        v.set(
            r#"CDN_LOC"#,
            replace_text(
                &split_fields(
                    &[v.get(r#"cdn_node"#)].join(" "),
                    &[r#"-f2"#.to_owned(), r#"-d-"#.to_owned()],
                )?,
                &[r#"s/[^a-z]//g"#.to_owned()],
            )?,
        );
        v.set(
            r#"lineNo"#,
            replace_text(
                &split_fields(
                    &[v.get(r#"IATACode"#)].join(" "),
                    &[r#"-f3"#.to_owned(), r#"-d|"#.to_owned()],
                )?,
                &[
                    r#"-n"#.to_owned(),
                    [
                        r#"/"#.to_owned(),
                        v.get(r#"CDN_LOC"#).to_uppercase(),
                        r#"/="#.to_owned(),
                    ]
                    .concat(),
                ],
            )?,
        );
        v.set(
            r#"location"#,
            replace_text(
                &replace_text(
                    &split_fields(
                        &select_fields(
                            &[v.get(r#"IATACode"#)].join(" "),
                            &[[r#"NR=="#.to_owned(), v.get(r#"lineNo"#)].concat()],
                        )?,
                        &[r#"-f1"#.to_owned(), r#"-d|"#.to_owned()],
                    )?,
                    &[r#"-e"#.to_owned(), r#"s/^[[:space:]]*//"#.to_owned()],
                )?,
                &[r#"s/\s*$//"#.to_owned()],
            )?,
        );
        output = decode_escapes(
            &[[
                r#"\r YouTube CDN:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Yellow"#),
                v.get(r#"CDN_ISP"#),
                r#" in "#.to_owned(),
                v.get(r#"location"#),
                r#" ("#.to_owned(),
                v.get(r#"cdn_node"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (matches_pattern(&v.get(r#"cdn_node"#), &r#"*s*"#.to_owned())) {
        v.set(
            r#"CDN_LOC"#,
            split_fields(
                &split_fields(
                    &[v.get(r#"cdn_node"#)].join(" "),
                    &[r#"-f2"#.to_owned(), r#"-d-"#.to_owned()],
                )?,
                &[r#"-c1-3"#.to_owned()],
            )?,
        );
        v.set(
            r#"lineNo"#,
            replace_text(
                &split_fields(
                    &[v.get(r#"IATACode"#)].join(" "),
                    &[r#"-f3"#.to_owned(), r#"-d|"#.to_owned()],
                )?,
                &[
                    r#"-n"#.to_owned(),
                    [
                        r#"/"#.to_owned(),
                        v.get(r#"CDN_LOC"#).to_uppercase(),
                        r#"/="#.to_owned(),
                    ]
                    .concat(),
                ],
            )?,
        );
        v.set(
            r#"location"#,
            replace_text(
                &replace_text(
                    &split_fields(
                        &select_fields(
                            &[v.get(r#"IATACode"#)].join(" "),
                            &[[r#"NR=="#.to_owned(), v.get(r#"lineNo"#)].concat()],
                        )?,
                        &[r#"-f1"#.to_owned(), r#"-d|"#.to_owned()],
                    )?,
                    &[r#"-e"#.to_owned(), r#"s/^[[:space:]]*//"#.to_owned()],
                )?,
                &[r#"s/\s*$//"#.to_owned()],
            )?,
        );
        output = decode_escapes(
            &[[
                r#"\r YouTube CDN:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                v.get(r#"location"#),
                r#" ("#.to_owned(),
                v.get(r#"cdn_node"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r YouTube CDN:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

async fn mediaunlocktest_bahamutanime(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpdeviceid"#,
        ctx.request(Request {
            url: r#"https://ani.gamer.com.tw/ajax/getdeviceid.php"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: true,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpdeviceid"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Bahamut Anime:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        ctx.clear_cookies();
        return Ok(output);
    }
    v.set(
        r#"tempdeviceid"#,
        map_characters(
            &select_fields(
                &select_lines(
                    &json_field(&[v.get(r#"tmpdeviceid"#)].join(" "), &".")?,
                    &[r#"deviceid"#.to_owned()],
                )?,
                &[r#"{print $2}"#.to_owned()],
            )?,
            &[r#"-d"#.to_owned(), r#"""#.to_owned()],
        )?,
    );
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: [
                r#"https://ani.gamer.com.tw/ajax/token.php?adID=89422&sn=38832&device="#.to_owned(),
                v.get(r#"tempdeviceid"#),
            ]
            .concat(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: true,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    v.set(
        r#"tmpresult2"#,
        ctx.request(Request {
            url: [
                r#"https://ani.gamer.com.tw/ajax/token.php?adID=89422&sn=37783&device="#.to_owned(),
                v.get(r#"tempdeviceid"#),
            ]
            .concat(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: true,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned()))
        || (matches_pattern(&v.get(r#"tmpresult2"#), &r#"curl*"#.to_owned()))
    {
        output = decode_escapes(
            &[[
                r#"\r Bahamut Anime:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    ctx.clear_cookies();
    v.set(
        r#"result"#,
        select_lines(
            &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
            &[r#"animeSn"#.to_owned()],
        )?,
    );
    v.set(
        r#"result2"#,
        select_lines(
            &json_field(&[v.get(r#"tmpresult2"#)].join(" "), &".")?,
            &[r#"animeSn"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) && (!(v.get(r#"result2"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Bahamut Anime:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes (Region: TW)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (!(v.get(r#"result2"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Bahamut Anime:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes (Region: HK/MO)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r Bahamut Anime:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_bilibilitw(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"randsession"#, random_id());
    v.set(r#"result"#, ctx.request(Request { url: [r#"https://api.bilibili.com/pgc/player/web/playurl?avid=50762638&cid=100279344&qn=0&type=&otype=json&ep_id=268176&fourk=1&fnver=0&fnval=16&session="#.to_owned(), v.get(r#"randsession"#), r#"&module=bangumi"#.to_owned()].concat(), method: r#"GET"#.to_owned(), headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]], body: None, follow: true, headers_only: false, include_headers: false, fail_status: true, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (!matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned())) {
        v.set(
            r#"result"#,
            split_fields(
                &select_fields(
                    &take_lines(
                        &select_lines(
                            &json_field(&[v.get(r#"result"#)].join(" "), &".")?,
                            &[r#""code""#.to_owned()],
                        )?,
                        &[r#"-1"#.to_owned()],
                    )?,
                    &[r#"{print $2}"#.to_owned()],
                )?,
                &[r#"-d"#.to_owned(), r#","#.to_owned(), r#"-f1"#.to_owned()],
            )?,
        );
        if (v.get(r#"result"#) == r#"0"#.to_owned()) {
            output = decode_escapes(
                &[[
                    r#"\r Bilibili Taiwan Only:\t\t\t"#.to_owned(),
                    v.get(r#"Font_Green"#),
                    r#"Yes"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
        } else if (v.get(r#"result"#) == r#"-10403"#.to_owned()) {
            output = decode_escapes(
                &[[
                    r#"\r Bilibili Taiwan Only:\t\t\t"#.to_owned(),
                    v.get(r#"Font_Red"#),
                    r#"No"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
        } else {
            output = decode_escapes(
                &[[
                    r#"\r Bilibili Taiwan Only:\t\t\t"#.to_owned(),
                    v.get(r#"Font_Red"#),
                    r#"Failed"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#" "#.to_owned(),
                    v.get(r#"Font_SkyBlue"#),
                    r#"("#.to_owned(),
                    v.get(r#"result"#),
                    r#")"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
        }
    } else {
        output = decode_escapes(
            &[[
                r#"\r Bilibili Taiwan Only:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_fridayvideo(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"tmpresult"#, ctx.request(Request { url: r#"https://video.friday.tw/api2/streaming/get?streamingId=122581&streamingType=2&contentType=4&contentId=1&clientId="#.to_owned(), method: r#"GET"#.to_owned(), headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]], body: None, follow: true, headers_only: false, include_headers: false, fail_status: false, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Friday Video:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        map_characters(
            &json_field(&[v.get(r#"tmpresult"#)].join(" "), &r#".code"#.to_owned())?,
            &[r#"-d"#.to_owned(), r#"""#.to_owned()],
        )?,
    );
    if (v.get(r#"result"#) != r#"null"#.to_owned()) {
        if (v.get(r#"result"#) == r#"1006"#.to_owned()) {
            output = decode_escapes(
                &[[
                    r#"\r Friday Video:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Red"#),
                    r#"No"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        } else if (v.get(r#"result"#) == r#"0000"#.to_owned()) {
            output = decode_escapes(
                &[[
                    r#"\r Friday Video:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Green"#),
                    r#"Yes"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        } else {
            output = decode_escapes(
                &[[
                    r#"\r Friday Video:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Red"#),
                    r#"Unknown (Code: "#.to_owned(),
                    v.get(r#"result"#),
                    r#")"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        }
    } else {
        output = decode_escapes(
            &[[
                r#"\r Friday Video:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_hamivideo(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://hamivideo.hinet.net/api/play.do?id=OTT_VOD_0000249064&freeProduct=1"#
                .to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Hami Video:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"checkfailed"#,
        split_fields(
            &select_lines(
                &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
                &[r#"code"#.to_owned()],
            )?,
            &[r#"-f4"#.to_owned(), r#"-d""#.to_owned()],
        )?,
    );
    if (v.get(r#"checkfailed"#) == r#"06001-106"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Hami Video:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"checkfailed"#) == r#"06001-107"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Hami Video:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Hami Video:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_hbogo_asia(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"tmpresult"#, ctx.request(Request { url: r#"https://api2.hbogoasia.com/v1/geog?lang=undefined&version=0&bundleId=www.hbogoasia.com"#.to_owned(), method: r#"GET"#.to_owned(), headers: vec![], body: None, follow: false, headers_only: false, include_headers: false, fail_status: false, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if ((v.get(r#"tmpresult"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r HBO GO Asia:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
            &[r#"territory"#.to_owned()],
        )?,
    );
    if ((v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r HBO GO Asia:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (!(v.get(r#"result"#)).is_empty()) {
        v.set(
            r#"CountryCode"#,
            split_fields(
                &select_lines(
                    &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
                    &[r#"country"#.to_owned()],
                )?,
                &[r#"-f4"#.to_owned(), r#"-d""#.to_owned()],
            )?,
        );
        output = decode_escapes(
            &[[
                r#"\r HBO GO Asia:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes (Region: "#.to_owned(),
                v.get(r#"CountryCode"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r HBO GO Asia:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_kktv(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://api.kktv.me/v3/ipcheck"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r KKTV:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        split_fields(
            &select_lines(
                &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
                &[r#"country"#.to_owned()],
            )?,
            &[r#"-f4"#.to_owned(), r#"-d""#.to_owned()],
        )?,
    );
    if (v.get(r#"result"#) == r#"TW"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r KKTV:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r KKTV:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_linetv_tw(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"tmpresult"#, ctx.request(Request { url: r#"https://www.linetv.tw/api/part/11829/eps/1/part?chocomemberId=&appId=062097f1b1f34e11e7f82aag22000aee"#.to_owned(), method: r#"GET"#.to_owned(), headers: vec![], body: None, follow: false, headers_only: false, include_headers: false, fail_status: false, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r LineTV.TW:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        split_fields(
            &select_fields(
                &select_lines(
                    &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
                    &[r#"countryCode"#.to_owned()],
                )?,
                &[r#"{print $2}"#.to_owned()],
            )?,
            &[r#"-f1"#.to_owned(), r#"-d,"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        if (v.get(r#"result"#) == r#"228"#.to_owned()) {
            output = decode_escapes(
                &[[
                    r#"\r LineTV.TW:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Green"#),
                    r#"Yes"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        } else {
            output = decode_escapes(
                &[[
                    r#"\r LineTV.TW:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Red"#),
                    r#"No"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        }
    } else {
        output = decode_escapes(
            &[[
                r#"\r LineTV.TW:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Unexpected Result: "#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_litv(ctx: &mut Context) -> Result<String> {
    let mut request = Request::get("https://www.litv.tv/api/get-urls-no-auth");
    request.method = "POST".into();
    request.timeout = 10;
    request
        .headers
        .push(header_pair("Content-Type: application/json")?);
    request.body = Some(
        serde_json::json!({
            "AssetId": "iNEWS",
            "MediaType": "channel",
            "puid": uuid::Uuid::new_v4().to_string(),
        })
        .to_string(),
    );
    let body = ctx.request(request).await?;
    let result = match json_field(&body, ".error.code").as_deref() {
        Ok("42000087") => "No",
        Ok("42000075") if !ctx.denied => "Yes",
        _ => "Unknown",
    };
    Ok(format!("LiTV: {result}"))
}

async fn mediaunlocktest_myvideo(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://www.myvideo.net.tw/login.do"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{url_effective}\n"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r MyVideo:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r MyVideo:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"serviceAreaBlock"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r MyVideo:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r MyVideo:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r MyVideo:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed (Network Connection)"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

async fn mediaunlocktest_bilibilihkmctw(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"randsession"#, random_id());
    v.set(r#"result"#, ctx.request(Request { url: [r#"https://api.bilibili.com/pgc/player/web/playurl?avid=18281381&cid=29892777&qn=0&type=&otype=json&ep_id=183799&fourk=1&fnver=0&fnval=16&session="#.to_owned(), v.get(r#"randsession"#), r#"&module=bangumi"#.to_owned()].concat(), method: r#"GET"#.to_owned(), headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]], body: None, follow: true, headers_only: false, include_headers: false, fail_status: true, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (!matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned())) {
        v.set(
            r#"result"#,
            split_fields(
                &select_fields(
                    &take_lines(
                        &select_lines(
                            &json_field(&[v.get(r#"result"#)].join(" "), &".")?,
                            &[r#""code""#.to_owned()],
                        )?,
                        &[r#"-1"#.to_owned()],
                    )?,
                    &[r#"{print $2}"#.to_owned()],
                )?,
                &[r#"-d"#.to_owned(), r#","#.to_owned(), r#"-f1"#.to_owned()],
            )?,
        );
        if (v.get(r#"result"#) == r#"0"#.to_owned()) {
            output = decode_escapes(
                &[[
                    r#"\r BiliBili Hongkong/Macau/Taiwan:\t"#.to_owned(),
                    v.get(r#"Font_Green"#),
                    r#"Yes"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
        } else if (v.get(r#"result"#) == r#"-10403"#.to_owned()) {
            output = decode_escapes(
                &[[
                    r#"\r BiliBili Hongkong/Macau/Taiwan:\t"#.to_owned(),
                    v.get(r#"Font_Red"#),
                    r#"No"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
        } else {
            output = decode_escapes(
                &[[
                    r#"\r BiliBili Hongkong/Macau/Taiwan:\t"#.to_owned(),
                    v.get(r#"Font_Red"#),
                    r#"Failed"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#" "#.to_owned(),
                    v.get(r#"Font_SkyBlue"#),
                    r#"("#.to_owned(),
                    v.get(r#"result"#),
                    r#")"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
        }
    } else {
        output = decode_escapes(
            &[[
                r#"\r BiliBili Hongkong/Macau/Taiwan:\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_hoytv(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://hoytv-live-stream.hoy.tv/ch78/index-fhd.m3u8"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r HOY TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r HOY TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r HOY TV:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed (Network Connection)"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

async fn mediaunlocktest_mytvsuper(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        map_characters(
            &select_fields(
                &select_lines(
                    &json_field(
                        &ctx.request(Request {
                            url: r#"https://www.mytvsuper.com/api/auth/getSession/self/"#
                                .to_owned(),
                            method: r#"GET"#.to_owned(),
                            headers: vec![],
                            body: None,
                            follow: false,
                            headers_only: false,
                            include_headers: false,
                            fail_status: false,
                            discard_body: false,
                            timeout: 10,
                            writeout: String::new(),
                            cookie_store: false,
                            cookie_load: false,
                            retries: 0,
                            tls13: false,
                        })
                        .await?,
                        &".",
                    )?,
                    &[r#"region"#.to_owned()],
                )?,
                &[r#"{print $2}"#.to_owned()],
            )?,
            &[r#"-d"#.to_owned(), r#","#.to_owned()],
        )?,
    );
    if (v.get(r#"result"#) == r#"1"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r MyTVSuper:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r MyTVSuper:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r MyTVSuper:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed (Network Connection)"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

async fn mediaunlocktest_nbatv(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://www.nba.com/watch/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r NBA TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"Service is not available in your region"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r NBA TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r NBA TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_abematv_iptest(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tempresult"#,
        ctx.request(Request {
            url: r#"https://api.abema.io/v1/ip/check?device=android"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Dalvik"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tempresult"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r Abema.TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"tempresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Abema.TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        split_fields(
            &select_fields(
                &select_lines(
                    &json_field(&[v.get(r#"tempresult"#)].join(" "), &".")?,
                    &[r#"isoCountryCode"#.to_owned()],
                )?,
                &[r#"{print $2}"#.to_owned()],
            )?,
            &[r#"-f2"#.to_owned(), r#"-d""#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        if (v.get(r#"result"#) == r#"JP"#.to_owned()) {
            output = decode_escapes(
                &[[
                    r#"\r Abema.TV:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Green"#),
                    r#"Yes"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
        } else {
            output = decode_escapes(
                &[[
                    r#"\r Abema.TV:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Yellow"#),
                    r#"Oversea Only (Region: "#.to_owned(),
                    v.get(r#"result"#),
                    r#")"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
        }
    } else {
        output = decode_escapes(
            &[[
                r#"\r Abema.TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_animefesta(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result1"#,
        ctx.request(Request {
            url: r#"https://api-animefesta.iowl.jp/v1/titles/1"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![
                [r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)],
                header_pair(&r#"x-requested-with: XMLHttpRequest"#.to_owned())?,
            ],
            body: None,
            follow: true,
            headers_only: true,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"result1"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r AnimeFesta:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (v.get(r#"result1"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r AnimeFesta:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result1"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r AnimeFesta:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r AnimeFesta:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_danimestore(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://animestore.docomo.ne.jp/animestore/reg_pc"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if ((v.get(r#"tmpresult"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r D Anime Store:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"isBlocked"#,
        select_lines(&[v.get(r#"tmpresult"#)].join(" "), &[r#"海外"#.to_owned()])?,
    );
    if (!(v.get(r#"isBlocked"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r D Anime Store:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r D Anime Store:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_dmmtv(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"tmpresult"#, ctx.request(Request { url: r#"https://api.beacon.dmm.com/v1/streaming/start"#.to_owned(), method: r#"POST"#.to_owned(), headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]], body: Some([r#"{"player_name":"dmmtv_browser","player_version":"0.0.0","content_type_detail":"VOD_SVOD","content_id":"11uvjcm4fw2wdu7drtd1epnvz","purchase_product_id":null}"#.to_owned()].join("&")), follow: false, headers_only: false, include_headers: false, fail_status: false, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r DMM TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"checkfailed"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"FOREIGN"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"checkfailed"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r DMM TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"checksuccess"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"UNAUTHORIZED"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"checksuccess"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r DMM TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r DMM TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Unsupported"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_erogamespace(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        select_lines(
            &ctx.request(Request {
                url: r#"https://erogamescape.org"#.to_owned(),
                method: r#"GET"#.to_owned(),
                headers: vec![],
                body: None,
                follow: true,
                headers_only: false,
                include_headers: false,
                fail_status: false,
                discard_body: false,
                timeout: 3,
                writeout: String::new(),
                cookie_store: false,
                cookie_load: false,
                retries: 0,
                tls13: false,
            })
            .await?,
            &[r#"18歳"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r EroGameSpace:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if ((v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r EroGameSpace:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_fod(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://geocontrol1.stream.ne.jp/fod-geo/check.xml?time=1624504256"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r FOD(Fuji TV):\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set("?", if (select_lines(&[v.get(r#"tmpresult"#)].join(" "), &[r#"true"#.to_owned()])?).is_empty() { "1" } else { "0" }.to_owned());
    if (number(&v.get(r#"?"#)) == number(&r#"0"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r FOD(Fuji TV):\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r FOD(Fuji TV):\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_hulujp(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        select_lines(
            &ctx.request(Request {
                url: r#"https://id.hulu.jp"#.to_owned(),
                method: r#"GET"#.to_owned(),
                headers: vec![
                    [r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)],
                    header_pair(&r#"Priority: u=1"#.to_owned())?,
                    header_pair(&r#"Accept: */*;q=0.8"#.to_owned())?,
                    header_pair(&r#"Accept-Language: en-US,en;q=0.5"#.to_owned())?,
                    header_pair(&r#"Accept-Encoding: none"#.to_owned())?,
                    header_pair(&r#"Sec-Ch-Ua-Mobile: ?0"#.to_owned())?,
                    header_pair(&r#"Sec-Ch-Ua-Platform: ?0"#.to_owned())?,
                    header_pair(&r#"Sec-Fetch-Dest: document"#.to_owned())?,
                    header_pair(&r#"Sec-Fetch-Mode: navigate"#.to_owned())?,
                    header_pair(&r#"Sec-Fetch-Site: none"#.to_owned())?,
                    header_pair(&r#"Sec-Fetch-User: ?1"#.to_owned())?,
                    header_pair(&r#"Upgrade-Insecure-Requests: 1"#.to_owned())?,
                ],
                body: None,
                follow: true,
                headers_only: false,
                include_headers: false,
                fail_status: false,
                discard_body: true,
                timeout: 10,
                writeout: r#"%{url_effective}%{http_code}\n"#.to_owned(),
                cookie_store: false,
                cookie_load: false,
                retries: 0,
                tls13: false,
            })
            .await?,
            &[r#"-E"#.to_owned(), r#"restrict|403|000"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Hulu Japan:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Hulu Japan:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Hulu Japan:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed (Network Connection)"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

async fn mediaunlocktest_j_com_on_demand(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://id.zaq.ne.jp"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r J:com On Demand:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"404"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r J:com On Demand:\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r J:com On Demand:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r J:com On Demand:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Unexpected Result: "#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_kancolle(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"http://w00g.kancolle-server.com/kcscontents/news/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Dalvik"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Kancolle Japan:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Kancolle Japan:\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Kancolle Japan:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r Kancolle Japan:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Unexpected Result: "#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_dam(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"http://cds1.clubdam.com/vhls-cds1/site/xbox/sample_1.mp4.m3u8"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Karaoke@DAM:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Karaoke@DAM:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Karaoke@DAM:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r Karaoke@DAM:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Unexpected Result: "#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_konosubafd(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://api.konosubafd.jp/api/masterlist"#.to_owned(),
            method: r#"POST"#.to_owned(),
            headers: vec![[
                r#"user-agent"#.to_owned(),
                r#"User-Agent: pj0007/212 CFNetwork/1240.0.4 Darwin/20.6.0"#.to_owned(),
            ]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Konosuba Fantastic Days:\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Konosuba Fantastic Days:\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Konosuba Fantastic Days:\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r Konosuba Fantastic Days:\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Unexpected Result: "#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_lemino(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://if.lemino.docomo.ne.jp/v1/user/delivery/watch/ready"#.to_owned(),
            method: r#"POST"#.to_owned(),
            headers: vec![],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Lemino:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"*CloudFront*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Lemino:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No  (Blocked)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        map_characters(
            &json_field(
                &[v.get(r#"tmpresult"#)].join(" "),
                &r#".result_code"#.to_owned(),
            )?,
            &[r#"-d"#.to_owned(), r#"""#.to_owned()],
        )?,
    );
    if (v.get(r#"result"#) != r#"null"#.to_owned()) {
        if (v.get(r#"result"#) == r#"WEBW100100"#.to_owned()) {
            output = decode_escapes(
                &[[
                    r#"\r Lemino:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Red"#),
                    r#"No"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        } else if (v.get(r#"result"#) == r#"WEBW300100"#.to_owned()) {
            output = decode_escapes(
                &[[
                    r#"\r Lemino:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Green"#),
                    r#"Yes"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        } else {
            output = decode_escapes(
                &[[
                    r#"\r Lemino:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Red"#),
                    r#"Unknown (Code: "#.to_owned(),
                    v.get(r#"result"#),
                    r#")"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        }
    } else {
        output = decode_escapes(
            &[[
                r#"\r Lemino:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_mgstage(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://www.mgstage.com/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: false,
            headers_only: true,
            include_headers: false,
            fail_status: false,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r MGStage:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r MGStage:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r MGStage:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r MGStage:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed ("#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_mora(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"result"#, ctx.request(Request { url: format!(r#"https://mora.jp/buy?__requestToken={}&returnUrl=https%3A%2F%2Fmora.jp%2Fpackage%2F43000087%2FTFDS01006B00Z%2F%3Ffmid%3DTOPRNKS%26trackMaterialNo%3D31168909&fromMoraUx=false&deleteMaterial="#, timestamp(true)), method: r#"GET"#.to_owned(), headers: vec![header_pair(&r#"host: mora.jp"#.to_owned())?], body: None, follow: true, headers_only: false, include_headers: false, fail_status: true, discard_body: true, timeout: 10, writeout: r#"%{http_code}"#.to_owned(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Mora:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Mora:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"500"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Mora:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Mora:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_music_jp(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://overseaauth.music-book.jp/globalIpcheck.js"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r music.jp:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r music.jp:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_nhkplus(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://location-plus.nhk.jp/geoip/area.json"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r NHK+:\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r NHK+:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result1"#,
        split_fields(
            &select_fields(
                &select_lines(
                    &json_field(&[v.get(r#"result"#)].join(" "), &".")?,
                    &[r#"country_code"#.to_owned()],
                )?,
                &[r#"{print $2}"#.to_owned()],
            )?,
            &[
                r#"-d"#.to_owned(),
                r#"""#.to_owned(),
                r#"-f"#.to_owned(),
                r#"2"#.to_owned(),
            ],
        )?,
    );
    if (v.get(r#"result1"#) == r#"JP"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r NHK+:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r NHK+:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r NHK+:\t\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_niconico(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://www.nicovideo.jp/watch/so23017073"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: false,
            headers_only: true,
            include_headers: false,
            fail_status: false,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Niconico:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (v.get(r#"result"#) == r#"400"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Niconico:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Niconico:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Niconico:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed ("#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_paravi(ctx: &mut Context) -> Result<String> {
    let mut request = Request::get("https://api.paravi.jp/api/v1/playback/auth");
    request.method = "POST".into();
    request.timeout = 10;
    request
        .headers
        .push(header_pair("Content-Type: application/json")?);
    request.body = Some(
        serde_json::json!({
            "meta_id": 17414,
            "vuid": random_id(),
            "device_code": 1,
            "app_id": 1,
        })
        .to_string(),
    );
    let body = ctx.request(request).await?;
    let result = match json_field(&body, ".type").as_deref() {
        Ok(r#""Unauthorized""#) => "Web Reachable",
        _ => "Unknown",
    };
    Ok(format!("Paravi: {result}"))
}

async fn mediaunlocktest_umajp(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://api-umamusume.cygames.jp/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Dalvik"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Pretty Derby Japan:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"404"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Pretty Derby Japan:\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Pretty Derby Japan:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r Pretty Derby Japan:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Unexpected Result: "#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_pcrjp(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://api-priconne-redive.cygames.jp/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Dalvik"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Princess Connect Re:Dive Japan:\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"404"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Princess Connect Re:Dive Japan:\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Princess Connect Re:Dive Japan:\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r Princess Connect Re:Dive Japan:\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Unexpected Result: "#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_projectsekai(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"result"#, ctx.request(Request { url: r#"https://game-version.sekai.colorfulpalette.org/1.8.1/3ed70b6a-8352-4532-b819-108837926ff5"#.to_owned(), method: r#"GET"#.to_owned(), headers: vec![[r#"user-agent"#.to_owned(), r#"User-Agent: pjsekai/48 CFNetwork/1240.0.4 Darwin/20.6.0"#.to_owned()]], body: None, follow: true, headers_only: false, include_headers: false, fail_status: true, discard_body: true, timeout: 10, writeout: r#"%{http_code}"#.to_owned(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Project Sekai: Colorful Stage:\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Project Sekai: Colorful Stage:\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Project Sekai: Colorful Stage:\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r Project Sekai: Colorful Stage:\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Unexpected Result: "#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_radiko(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://radiko.jp/area?_=1625406539531"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Radiko:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"checkfailed"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"class="OUT""#.to_owned()],
        )?,
    );
    if (!(v.get(r#"checkfailed"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Radiko:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"checksuccess"#,
        select_lines(&[v.get(r#"tmpresult"#)].join(" "), &[r#"JAPAN"#.to_owned()])?,
    );
    if (!(v.get(r#"checksuccess"#)).is_empty()) {
        v.set(
            r#"area"#,
            replace_text(
                &select_fields(
                    &[v.get(r#"tmpresult"#)].join(" "),
                    &[r#"{print $2}"#.to_owned()],
                )?,
                &[r#"s/.*>//"#.to_owned()],
            )?,
        );
        output = decode_escapes(
            &[[
                r#"\r Radiko:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes (City: "#.to_owned(),
                v.get(r#"area"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Radiko:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Unsupported"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_rakutenmagazine(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url:
                r#"https://data-cloudauthoring.magazine.rakuten.co.jp/rem_repository/////////.key"#
                    .to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Rakuten MAGAZINE:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"404"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Rakuten MAGAZINE:\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Rakuten MAGAZINE:\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed ("#.to_owned(),
            v.get(r#"result"#),
            r#")"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

async fn mediaunlocktest_rakutentvjp(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://api.tv.rakuten.co.jp/content/playinfo.json?content_id=1&device_id=1"#
                .to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Rakuten TV JP:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        map_characters(
            &json_field(
                &[v.get(r#"tmpresult"#)].join(" "),
                &r#".result.code"#.to_owned(),
            )?,
            &[r#"-d"#.to_owned(), r#"""#.to_owned()],
        )?,
    );
    if (v.get(r#"result"#) == r#"40404030802"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Rakuten TV JP:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"40301720109"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Rakuten TV JP:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Rakuten TV JP:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Unknown (Code: "#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_telasa(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://api-videopass-anon.kddi-video.com/v1/playback/system_status"#
                .to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![header_pair(&format!(
                "X-Device-ID: {}",
                uuid::Uuid::new_v4().hyphenated()
            ))?],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Telasa:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"isForbidden"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"IPLocationNotAllowed"#.to_owned()],
        )?,
    );
    v.set(
        r#"isAllowed"#,
        split_fields(
            &select_lines(
                &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
                &[r#""type""#.to_owned()],
            )?,
            &[r#"-f4"#.to_owned(), r#"-d""#.to_owned()],
        )?,
    );
    if (!(v.get(r#"isForbidden"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Telasa:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if ((v.get(r#"isForbidden"#)).is_empty())
        && (v.get(r#"isAllowed"#) == r#"OK"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r Telasa:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Telasa:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed "#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

async fn mediaunlocktest_unext(ctx: &mut Context) -> Result<String> {
    let mut request = Request::get(
        "https://cc.unext.jp/?operationName=cosmo_getPlaylistUrl&variables=%7B%22code%22%3A%22ED00440417%22%2C%22playMode%22%3A%22caption%22%2C%22bitrateLow%22%3A192%2C%22bitrateHigh%22%3Anull%2C%22validationOnly%22%3Afalse%7D&extensions=%7B%22persistedQuery%22%3A%7B%22version%22%3A1%2C%22sha256Hash%22%3A%22a2309e22a6819ff747cf9a389dd78db35fa3c386fac1d53461061ba20fa44e34%22%7D%7D",
    );
    request.timeout = 10;
    request
        .headers
        .push(header_pair("Content-Type: application/json")?);
    let body = ctx.request(request).await?;
    let parsed = serde_json::from_str::<serde_json::Value>(&body).ok();
    let status = parsed.as_ref().and_then(|root| {
        root.get("resultStatus").or_else(|| {
            root.get("data")?
                .as_object()?
                .values()
                .find_map(|value| value.get("resultStatus"))
        })
    });
    let result = match status.and_then(serde_json::Value::as_u64) {
        Some(200) if !ctx.denied => "Yes",
        Some(475) if !ctx.denied => "Web Reachable",
        Some(467) => "No",
        _ => "Unknown",
    };
    Ok(format!("U-NEXT: {result}"))
}

async fn mediaunlocktest_videomarket(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"TokenSrc"#,
        ctx.request(Request {
            url: r#"https://www.videomarket.jp/player/17588S/A17588S001999H01"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"TokenSrc"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r VideoMarket:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"Token"#,
        select_lines(
            &[v.get(r#"TokenSrc"#)].join(" "),
            &[
                r#"-Eo"#.to_owned(),
                r#"notLoggedInTokenPc:(.|)*notLoggedInTokenSpAndroid"#.to_owned(),
            ],
        )?,
    );
    v.set(r#"tmpresult"#, ctx.request(Request { url: r#"https://www.videomarket.jp/graphql"#.to_owned(), method: r#"POST"#.to_owned(), headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)], header_pair(&[r#"Authorization:Bearer "#.to_owned(), slice_text(&v.get(r#"Token"#), 20, Some(-27))].concat())?], body: Some([r#"{"operationName":"readStatus","variables":{},"query":"query readStatus {\n  readStatus {\n    isRead\n    __typename\n  }\n}\n"}"#.to_owned()].join("&")), follow: true, headers_only: false, include_headers: false, fail_status: true, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r VideoMarket:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"OverseasAccess"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r VideoMarket:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r VideoMarket:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_wowow(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://mapi.wowow.co.jp/api/v1/playback/auth"#.to_owned(),
            method: r#"POST"#.to_owned(),
            headers: vec![
                [r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)],
                header_pair(&r#"Content-Type: application/json"#.to_owned())?,
            ],
            body: Some([r#"{"meta_id":81174}"#.to_owned()].join("&")),
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r WOWOW:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"checkfailed"#,
        json_field(
            &[v.get(r#"tmpresult"#)].join(" "),
            &r#".error.code"#.to_owned(),
        )?,
    );
    if (v.get(r#"checkfailed"#) == r#"2055"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r WOWOW:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"checkfailed"#) == r#"2041"#.to_owned())
        || (v.get(r#"checkfailed"#) == r#"2003"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r WOWOW:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r WOWOW:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Unknown (Code: "#.to_owned(),
                v.get(r#"checkfailed"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_aetv(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://ccpa-service.sp-prod.net/ccpa/consent/10265/display-dns"#.to_owned(),
            method: r#"POST"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r A&E TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r A&E TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_fields(
            &split_fields(
                &select_lines(
                    &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
                    &[r#""ccpaApplies":"#.to_owned()],
                )?,
                &[r#"-f1"#.to_owned(), r#"-d,"#.to_owned()],
            )?,
            &[r#"{print $2}"#.to_owned()],
        )?,
    );
    if (v.get(r#"result"#) == r#"true"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r A&E TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"false"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r A&E TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r A&E TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_acorntv(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://acorn.tv/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    v.set(
        r#"isblocked"#,
        ctx.request(Request {
            url: r#"https://acorn.tv/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if ((v.get(r#"tmpresult"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Acorn TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"isblocked"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Acorn TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"Not yet available in your country"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Acorn TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Acorn TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_britbox(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://www.britbox.com/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{url_effective}\n"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r BritBox:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"locationnotvalidated"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        v.set(
            r#"region"#,
            split_fields(
                &[v.get(r#"tmpresult"#)].join(" "),
                &[r#"-d"#.to_owned(), r#"/"#.to_owned(), r#"-f4"#.to_owned()],
            )?,
        );
        output = decode_escapes(
            &[[
                r#"\r BritBox:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No  (Region: "#.to_owned(),
                v.get(r#"region"#).to_uppercase(),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r BritBox:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r BritBox:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed (Network Connection)"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

async fn mediaunlocktest_cbcgem(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://www.cbc.ca/g/stats/js/cbc-stats-top.js"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if ((v.get(r#"tmpresult"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r CBC Gem:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        split_fields(
            &split_fields(
                &replace_text(
                    &[v.get(r#"tmpresult"#)].join(" "),
                    &[r#"s/.*country":"//"#.to_owned()],
                )?,
                &[r#"-f1"#.to_owned(), r#"-d}"#.to_owned()],
            )?,
            &[r#"-f1"#.to_owned(), r#"-d""#.to_owned()],
        )?,
    );
    if (v.get(r#"result"#) == r#"CA"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r CBC Gem:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r CBC Gem:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_crackle(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        select_lines(
            &ctx.request(Request {
                url: r#"https://prod-api.crackle.com/appconfig"#.to_owned(),
                method: r#"HEAD"#.to_owned(),
                headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
                body: None,
                follow: false,
                headers_only: true,
                include_headers: false,
                fail_status: false,
                discard_body: false,
                timeout: 10,
                writeout: String::new(),
                cookie_store: false,
                cookie_load: false,
                retries: 0,
                tls13: false,
            })
            .await?,
            &[r#"-Ei"#.to_owned(), r#"x-crackle-region:|curl"#.to_owned()],
        )?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Crackle:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if ((v.get(r#"tmpresult"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Crackle:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        replace_text(
            &select_fields(
                &[v.get(r#"tmpresult"#)].join(" "),
                &[r#"{print $2}"#.to_owned()],
            )?,
            &[r#"s/[[:space:]]//g"#.to_owned()],
        )?,
    );
    if (v.get(r#"result"#) == r#"US"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Crackle:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r Crackle:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_crave(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"tmpresult"#, ctx.request(Request { url: r#"https://capi.9c9media.com/destinations/crave_atexace/platforms/desktop/playback/contents/2189628/contentPackages/4178863/manifest.pmpd"#.to_owned(), method: r#"GET"#.to_owned(), headers: vec![], body: None, follow: false, headers_only: false, include_headers: false, fail_status: false, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if ((v.get(r#"tmpresult"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Crave:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"Geo Constraint Restrictions"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Crave:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Crave:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_crunchyroll(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://c.evidon.com/geo/country.js"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r Crunchyroll:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Crunchyroll:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"'code':'us'"#.to_owned()],
        )?,
    );
    if ((v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Crunchyroll:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r Crunchyroll:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_cwtv(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://www.cwtv.com/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 3,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r CW TV:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r CW TV:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r CW TV:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r CW TV:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Unexpected Result: "#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_attnow(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://www.atttvnow.com/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Directv Stream:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Directv Stream:\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Directv Stream:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_discoveryplus(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"GetToken"#, ctx.request(Request { url: format!(r#"https://us1-prod-direct.discoveryplus.com/token?deviceId={}&realm=go&shortlived=true"#, random_id()), method: r#"GET"#.to_owned(), headers: vec![], body: None, follow: false, headers_only: false, include_headers: false, fail_status: false, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (matches_pattern(&v.get(r#"GetToken"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r Discovery+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"GetToken"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Discovery+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"Token"#,
        split_fields(
            &select_lines(
                &json_field(&[v.get(r#"GetToken"#)].join(" "), &".")?,
                &[r#""token":"#.to_owned()],
            )?,
            &[r#"-f4"#.to_owned(), r#"-d""#.to_owned()],
        )?,
    );
    if v.get(r#"Token"#).is_empty() {
        return Ok("Discovery+: Failed (Unexpected Result)".to_owned());
    }
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://us1-prod-direct.discoveryplus.com/users/me"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[
                r#"cookie"#.to_owned(),
                [r#"st="#.to_owned(), v.get(r#"Token"#)].concat(),
            ]],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    v.set(
        r#"result"#,
        split_fields(
            &select_lines(
                &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
                &[r#"currentLocationTerritory"#.to_owned()],
            )?,
            &[r#"-f4"#.to_owned(), r#"-d""#.to_owned()],
        )?,
    );
    if v.get(r#"result"#).is_empty() {
        return Ok("Discovery+: Failed (Unexpected Result)".to_owned());
    }
    if (v.get(r#"result"#) == r#"us"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Discovery+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Discovery+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Discovery+:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed "#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

const TS_PACKET_SIZE: usize = 188;
const FOX_TS_PREFIX: usize = TS_PACKET_SIZE * 3;
const FOX_HLS_PREFIX: usize = 8 * 1024;

async fn mediaunlocktest_fox(ctx: &mut Context) -> Result<String> {
    let mut request = Request::get(
        "https://x-live-fox-stgec.uplynk.com/ausw/slices/8d1/d8e6eec26bf544f084bad49a7fa2eac5/8d1de292bcc943a6b886d029e6c0dc87/G00000000.ts?cloud=aws&si=0",
    );
    request.timeout = 10;
    request.follow = true;
    let prefix = PrefixRead {
        status: StatusCode::OK,
        limit_for_first_byte: |first| {
            if first == b'#' {
                FOX_HLS_PREFIX
            } else {
                FOX_TS_PREFIX
            }
        },
    };
    if let Ok(response) = ctx.request_bytes(request, Some(prefix)).await {
        ctx.failed |= response.status.as_u16() >= 400;
        if response.status == StatusCode::OK
            && (fox_ts_prefix(&response.body) || fox_hls_prefix(&response))
        {
            return Ok("FOX: Yes".into());
        }
    }
    Ok("FOX: Unknown".into())
}

fn fox_ts_prefix(body: &[u8]) -> bool {
    body.len() == FOX_TS_PREFIX
        && body.chunks_exact(TS_PACKET_SIZE).all(|packet| {
            packet[0] == 0x47
                && packet[1] & 0x80 == 0
                && match (packet[3] >> 4) & 0x03 {
                    1 => true,
                    2 => usize::from(packet[4]) == TS_PACKET_SIZE - 5,
                    3 => usize::from(packet[4]) < TS_PACKET_SIZE - 5,
                    _ => false,
                }
        })
}

fn fox_hls_prefix(response: &ByteResponse) -> bool {
    let body = &response.body;
    let complete_bytes = if response.end == BodyEnd::Complete {
        body.len()
    } else {
        body.iter()
            .rposition(|&byte| byte == b'\n')
            .map_or(0, |at| at + 1)
    };
    let Ok(playlist) = std::str::from_utf8(&body[..complete_bytes]) else {
        return false;
    };
    let mut lines = playlist.lines();
    if lines.next() != Some("#EXTM3U") {
        return false;
    }
    let mut expects_uri = false;
    for line in lines {
        if let Some(info) = line.strip_prefix("#EXTINF:") {
            expects_uri = info.split_once(',').is_some_and(|(duration, _)| {
                duration
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'.')
                    && duration.parse::<f64>().is_ok_and(f64::is_finite)
            });
        } else if let Some(attributes) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            expects_uri = attributes
                .split('"')
                .step_by(2)
                .flat_map(|attributes| attributes.split(','))
                .filter_map(|attribute| attribute.strip_prefix("BANDWIDTH="))
                .any(|value| {
                    value.bytes().all(|byte| byte.is_ascii_digit())
                        && value.parse::<u64>().is_ok_and(|bandwidth| bandwidth > 0)
                });
        } else if !line.is_empty()
            && !line.starts_with('#')
            && std::mem::take(&mut expects_uri)
            && fox_hls_uri(&response.url, line)
        {
            return true;
        }
    }
    false
}

fn fox_hls_uri(base: &Url, uri: &str) -> bool {
    if !uri.contains(['.', '/', '?'])
        || !uri
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._~:/?[]@!$&'()*+,;=%".contains(&byte))
        || uri.bytes().enumerate().any(|(index, byte)| {
            byte == b'%'
                && !uri
                    .as_bytes()
                    .get(index + 1..index + 3)
                    .is_some_and(|digits| digits.iter().all(u8::is_ascii_hexdigit))
        })
    {
        return false;
    }
    base.join(uri).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && (!url.path().ends_with('/') || url.query().is_some_and(|query| !query.is_empty()))
    })
}

async fn mediaunlocktest_fubotv(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"radom_num"#, random_digit());
    v.set(r#"tmpresult"#, ctx.request(Request { url: [r#"https://api.fubo.tv/appconfig/v1/homepage?platform=web&client_version=R20230310."#.to_owned(), v.get(r#"radom_num"#), r#"&nav=v02>&1"#.to_owned()].concat(), method: r#"GET"#.to_owned(), headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]], body: None, follow: true, headers_only: false, include_headers: false, fail_status: false, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r Fubo TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Fubo TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"Forbidden IP"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Fubo TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r Fubo TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_fxnow(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://fxnow.fxnetworks.com/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r FXNOW:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r FXNOW:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"is not accessible"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r FXNOW:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r FXNOW:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_kbsamerican(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"tmpresult"#, ctx.request(Request { url: r#"https://vod.kbs.co.kr/index.html?source=episode&sname=vod&stype=vod&program_code=T2022-0690&program_id=PS-2022164275-01-000&broadcast_complete_yn=N&local_station_code=00&section_code=03"#.to_owned(), method: r#"GET"#.to_owned(), headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]], body: None, follow: true, headers_only: false, include_headers: false, fail_status: true, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r KBS American:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r KBS American:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result1"#,
        select_lines(
            &select_lines(&[v.get(r#"tmpresult"#)].join(" "), &[r#"ipck"#.to_owned()])?,
            &[r#"American\\": true"#.to_owned()],
        )?,
    );
    if ((v.get(r#"result1"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r KBS American:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r KBS American:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_kocowa(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result1"#,
        ctx.request(Request {
            url: r#"https://www.kocowa.com/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result1"#) == r#"000"#.to_owned()) && (v.get(r#"1"#) == r#"6"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r KOCOWA:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result1"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r KOCOWA:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (v.get(r#"result1"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r KOCOWA:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r KOCOWA:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_mathsspot(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult1"#,
        ctx.request(Request {
            url: r#"https://netv2.now.gg/v3/playtoken"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult1"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r Maths Spot:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"tmpresult1"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Maths Spot:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"blocked"#,
        select_lines(
            &[v.get(r#"tmpresult1"#)].join(" "),
            &[r#"Request blocked"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"blocked"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Maths Spot:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No  (Proxy/VPN Detected)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"playtoken"#,
        split_fields(
            &select_fields(
                &select_lines(
                    &json_field(&[v.get(r#"tmpresult1"#)].join(" "), &".")?,
                    &[r#""playToken":"#.to_owned()],
                )?,
                &[r#"{print $2}"#.to_owned()],
            )?,
            &[r#"-f2"#.to_owned(), r#"-d""#.to_owned()],
        )?,
    );
    v.set(
        r#"region"#,
        split_fields(
            &select_fields(
                &select_lines(
                    &json_field(&[v.get(r#"tmpresult1"#)].join(" "), &".")?,
                    &[r#""countryCode":"#.to_owned()],
                )?,
                &[r#"{print $2}"#.to_owned()],
            )?,
            &[r#"-f2"#.to_owned(), r#"-d""#.to_owned()],
        )?,
    );
    v.set(r#"host"#, ctx.request(Request { url: [r#"https://mathsspot.com/2/api/play/v1/startSession?appId=5349&initialOrientation=landscape&utmSource=NA&utmMedium=NA&utmCampaign=NA&deviceType=&playToken="#.to_owned(), v.get(r#"playtoken"#), r#"&deepLinkUrl=&accessCode="#.to_owned()].concat(), method: r#"GET"#.to_owned(), headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]], body: None, follow: false, headers_only: false, include_headers: false, fail_status: false, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    v.set(r#"tmpresult2"#, ctx.request(Request { url: [r#"https://mathsspot.com/2/api/play/v1/startSession?appId=5349&initialOrientation=landscape&utmSource=NA&utmMedium=NA&utmCampaign=NA&deviceType=&playToken="#.to_owned(), v.get(r#"playtoken"#), r#"&deepLinkUrl=&accessCode="#.to_owned()].concat(), method: r#"GET"#.to_owned(), headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)], header_pair(&[r#"x-ngg-fe-version: "#.to_owned(), v.get(r#"host"#)].concat())?], body: None, follow: false, headers_only: false, include_headers: false, fail_status: false, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (matches_pattern(&v.get(r#"host"#), &r#"curl*"#.to_owned()))
        || (matches_pattern(&v.get(r#"tmpresult2"#), &r#"curl*"#.to_owned()))
    {
        output = decode_escapes(
            &[[
                r#"\r Maths Spot:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        split_fields(
            &select_fields(
                &select_lines(
                    &json_field(&[v.get(r#"tmpresult2"#)].join(" "), &".")?,
                    &[r#""status":"#.to_owned()],
                )?,
                &[r#"{print $2}"#.to_owned()],
            )?,
            &[r#"-f2"#.to_owned(), r#"-d""#.to_owned()],
        )?,
    );
    if (v.get(r#"result"#) == r#"FailureServiceNotInRegion"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Maths Spot:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"Success"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Maths Spot:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes (Region: "#.to_owned(),
                v.get(r#"region"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r Maths Spot:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_nbctv(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        "onetrustresult",
        ctx.request(Request::get(
            "https://geolocation.onetrust.com/cookieconsentpub/v1/geo/location/dnsfeed",
        ))
        .await?,
    );
    if (matches_pattern(&v.get(r#"onetrustresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r NBC TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if ((v.get(r#"onetrustresult"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r NBC TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"onetrustresult"#)].join(" "),
            &[r#""country":"US""#.to_owned()],
        )?,
    );
    if ((v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r NBC TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r NBC TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_nflplus(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://www.nfl.com/plus/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}\n%{url_effective}\n"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"000*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r NFL+:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"tmpresult"#), &r#"000*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r NFL+:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"nflgamepass"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r NFL+:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r NFL+:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_paramountplus(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://www.paramountplus.com/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}_%{url_effective}\n"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: true,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Paramount+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (matches_pattern(&v.get(r#"result"#), &r#"*intl*"#.to_owned()))
        || (matches_pattern(&v.get(r#"result"#), &r#"403*"#.to_owned()))
    {
        output = decode_escapes(
            &[[
                r#"\r Paramount+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"result"#), &r#"200*"#.to_owned())) {
        v.set(
            r#"region"#,
            split_fields(
                &[v.get(r#"result"#)].join(" "),
                &[r#"-f4"#.to_owned(), r#"-d"#.to_owned(), r#"/"#.to_owned()],
            )?,
        );
        if ((v.get(r#"region"#)).is_empty()) {
            output = decode_escapes(
                &[[
                    r#"\r Paramount+:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Green"#),
                    r#"Yes (Region: US)"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        } else {
            output = decode_escapes(
                &[[
                    r#"\r Paramount+:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Green"#),
                    r#"Yes (Region: "#.to_owned(),
                    v.get(r#"region"#).to_uppercase(),
                    r#")"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
            return Ok(output);
        }
    } else {
        output = decode_escapes(
            &[[
                r#"\r Paramount+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Unknown Resp)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Paramount+:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

async fn mediaunlocktest_peacocktv(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://www.peacocktv.com/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}\n%{url_effective}\n"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"000*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Peacock TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"unavailable"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Peacock TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r Peacock TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_philo(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://content-us-east-2-fastly-b.www.philo.com/geo"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r Philo:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Philo:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        replace_text(
            &select_fields(
                &split_fields(
                    &select_lines(
                        &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
                        &[r#""status":"#.to_owned()],
                    )?,
                    &[r#"-f1"#.to_owned(), r#"-d,"#.to_owned()],
                )?,
                &[r#"{print $2}"#.to_owned()],
            )?,
            &[r#"s/"//g"#.to_owned()],
        )?,
    );
    if (v.get(r#"result"#) == r#"FAIL"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Philo:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"SUCCESS"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Philo:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r Philo:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_plutotv(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://pluto.tv/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{url_effective}\n"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Pluto TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"thanks-for-watching"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Pluto TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Pluto TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Pluto TV:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed (Network Connection)"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

async fn mediaunlocktest_popcornflix(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"result"#, ctx.request(Request { url: r#"https://popcornflix-prod.cloud.seachange.com/cms/popcornflix/clientconfiguration/versions/2"#.to_owned(), method: r#"GET"#.to_owned(), headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]], body: None, follow: false, headers_only: false, include_headers: false, fail_status: false, discard_body: true, timeout: 10, writeout: r#"%{http_code}"#.to_owned(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (v.get(r#"result"#) == r#"000"#.to_owned()) && (v.get(r#"1"#) == r#"6"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Popcornflix:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"000"#.to_owned()) && (v.get(r#"1"#) == r#"4"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Popcornflix:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Popcornflix:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Popcornflix:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r Popcornflix:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Unexpected Result: "#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_slingtv(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://p-geo.movetv.com/geo"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Rakuten TV JP:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"is_restricted"#,
        map_characters(
            &json_field(
                &[v.get(r#"tmpresult"#)].join(" "),
                &r#".ip_restricted"#.to_owned(),
            )?,
            &[r#"-d"#.to_owned(), r#"""#.to_owned()],
        )?,
    );
    v.set(
        r#"region"#,
        map_characters(
            &json_field(
                &[v.get(r#"tmpresult"#)].join(" "),
                &r#".country"#.to_owned(),
            )?,
            &[r#"-d"#.to_owned(), r#"""#.to_owned()],
        )?,
    );
    if (v.get(r#"is_restricted"#) == r#"true"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Sling TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No  (Region: "#.to_owned(),
                v.get(r#"region"#).to_uppercase(),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Sling TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes (Region: "#.to_owned(),
                v.get(r#"region"#).to_uppercase(),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_showtime(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://www.showtime.com/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r SHOWTIME:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r SHOWTIME:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r SHOWTIME:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r SHOWTIME:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Unexpected Result: "#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_shudder(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://www.shudder.com/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Shudder:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"not available"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Shudder:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r Shudder:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_starz(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"authorization"#,
        ctx.request(Request {
            url:
                r#"https://www.starz.com/sapi/header/v1/starz/us/09b397fc9eb64d5080687fc8a218775b"#
                    .to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![
                [r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)],
                header_pair(&r#"Referer: https://www.starz.com/us/en/"#.to_owned())?,
            ],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://auth.starz.com/api/v4/User/geolocation"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![header_pair(
                &[
                    r#"AuthTokenAuthorization: "#.to_owned(),
                    v.get(r#"authorization"#),
                ]
                .concat(),
            )?],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if ((v.get(r#"tmpresult"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Starz:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"isAllowedAccess"#,
        split_fields(
            &select_fields(
                &select_lines(
                    &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
                    &[r#"isAllowedAccess"#.to_owned()],
                )?,
                &[r#"{print $2}"#.to_owned()],
            )?,
            &[r#"-f1"#.to_owned(), r#"-d,"#.to_owned()],
        )?,
    );
    v.set(
        r#"isAllowedCountry"#,
        split_fields(
            &select_fields(
                &select_lines(
                    &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
                    &[r#"isAllowedCountry"#.to_owned()],
                )?,
                &[r#"{print $2}"#.to_owned()],
            )?,
            &[r#"-f1"#.to_owned(), r#"-d,"#.to_owned()],
        )?,
    );
    v.set(
        r#"isKnownProxy"#,
        split_fields(
            &select_fields(
                &select_lines(
                    &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
                    &[r#"isKnownProxy"#.to_owned()],
                )?,
                &[r#"{print $2}"#.to_owned()],
            )?,
            &[r#"-f1"#.to_owned(), r#"-d,"#.to_owned()],
        )?,
    );
    if (v.get(r#"isAllowedAccess"#) == r#"true"#.to_owned())
        && (v.get(r#"isAllowedCountry"#) == r#"true"#.to_owned())
        && (v.get(r#"isKnownProxy"#) == r#"false"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r Starz:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"isAllowedAccess"#) == r#"false"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Starz:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"isKnownProxy"#) == r#"false"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Starz:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Starz:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_tlcgo(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        "onetrustresult",
        ctx.request(Request::get(
            "https://geolocation.onetrust.com/cookieconsentpub/v1/geo/location/dnsfeed",
        ))
        .await?,
    );
    v.set(
        r#"onetrustresult"#,
        ctx.request(Request {
            url: r#"https://geolocation.onetrust.com/cookieconsentpub/v1/geo/location/dnsfeed"#
                .to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"1"#) == r#"6"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r TLC GO:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"onetrustresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r TLC GO:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if ((v.get(r#"onetrustresult"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r TLC GO:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"onetrustresult"#)].join(" "),
            &[r#""country":"US""#.to_owned()],
        )?,
    );
    if ((v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r TLC GO:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r TLC GO:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_tubitv(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://tubitv.com"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{url_effective}\n"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Tubi TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"gdpr.tubi.tv"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Tubi TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r Tubi TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_directvgo(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://www.directvgo.com/registrarse"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{url_effective}\n"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r DirecTV Go:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"isForbidden"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"proximamente"#.to_owned()],
        )?,
    );
    v.set(
        r#"region"#,
        map_characters(
            &split_fields(
                &[v.get(r#"tmpresult"#)].join(" "),
                &[r#"-f4"#.to_owned(), r#"-d/"#.to_owned()],
            )?,
            &[r#"[:lower:]"#.to_owned(), r#"[:upper:]"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"isForbidden"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r DirecTV Go:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if ((v.get(r#"isForbidden"#)).is_empty()) && (!(v.get(r#"region"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r DirecTV Go:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes (Region: "#.to_owned(),
                v.get(r#"region"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r DirecTV Go:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed "#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

async fn mediaunlocktest_amediateka(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://www.amediateka.ru/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: true,
            include_headers: false,
            fail_status: false,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{url_effective}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"*curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r Amediateka:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"tmpresult"#), &r#"*curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Amediateka:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"unavailable"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Amediateka:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Amediateka:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_bbciplayer(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"tmpresult"#, ctx.request(Request { url: r#"https://open.live.bbc.co.uk/mediaselector/6/select/version/2.0/mediaset/pc/vpid/bbc_one_london/format/json/jsfunc/JS_callbacks0"#.to_owned(), method: r#"GET"#.to_owned(), headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]], body: None, follow: true, headers_only: false, include_headers: false, fail_status: true, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (v.get(r#"tmpresult"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r BBC iPLAYER:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (!(v.get(r#"tmpresult"#)).is_empty()) {
        v.set(
            r#"result"#,
            select_lines(
                &[v.get(r#"tmpresult"#)].join(" "),
                &[r#"geolocation"#.to_owned()],
            )?,
        );
        if (!(v.get(r#"result"#)).is_empty()) {
            output = decode_escapes(
                &[[
                    r#"\r BBC iPLAYER:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Red"#),
                    r#"No"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
        } else {
            output = decode_escapes(
                &[[
                    r#"\r BBC iPLAYER:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Green"#),
                    r#"Yes"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
        }
    } else {
        output = decode_escapes(
            &[[
                r#"\r BBC iPLAYER:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_canalplus(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://boutique-tunnel.canalplus.com/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: true,
            include_headers: false,
            fail_status: false,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{url_effective}\n"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Canal+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"other-country-blocking"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Canal+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Canal+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Canal+:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed (Network Connection)"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

async fn mediaunlocktest_channel4(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://www.channel4.com/simulcast/channels/C4"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Channel 4:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Channel 4:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Channel 4:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed (Network Connection)"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

async fn mediaunlocktest_discoveryplusuk(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"GetToken"#, ctx.request(Request { url: format!(r#"https://disco-api.discoveryplus.co.uk/token?realm=questuk&deviceId={}{}&shortlived=true"#, random_id(), random_id()), method: r#"GET"#.to_owned(), headers: vec![], body: None, follow: false, headers_only: false, include_headers: false, fail_status: false, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (matches_pattern(&v.get(r#"GetToken"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r Discovery+ UK:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"GetToken"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Discovery+ UK:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"Token"#,
        split_fields(
            &select_lines(
                &json_field(&[v.get(r#"GetToken"#)].join(" "), &".")?,
                &[r#""token":"#.to_owned()],
            )?,
            &[r#"-f4"#.to_owned(), r#"-d""#.to_owned()],
        )?,
    );
    if v.get(r#"Token"#).is_empty() {
        return Ok("Discovery+ UK: Failed (Unexpected Result)".to_owned());
    }
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://disco-api.discoveryplus.co.uk/users/me"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[
                r#"cookie"#.to_owned(),
                [r#"st="#.to_owned(), v.get(r#"Token"#)].concat(),
            ]],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    v.set(
        r#"result"#,
        split_fields(
            &select_lines(
                &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
                &[r#"currentLocationTerritory"#.to_owned()],
            )?,
            &[r#"-f4"#.to_owned(), r#"-d""#.to_owned()],
        )?,
    );
    if v.get(r#"result"#).is_empty() {
        return Ok("Discovery+ UK: Failed (Unexpected Result)".to_owned());
    }
    if (v.get(r#"result"#) == r#"gb"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Discovery+ UK:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Discovery+ UK:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Discovery+ UK:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed "#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

async fn mediaunlocktest_eurosport(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://www.eurosport.com/"#.to_owned(),
            method: r#"HEAD"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: false,
            headers_only: true,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Eurosports Region:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"region"#,
        select_lines(
            &select_lines(
                &[v.get(r#"result"#)].join(" "),
                &[r#"eurosport_country_code"#.to_owned()],
            )?,
            &[
                r#"-Eo"#.to_owned(),
                r#"eurosport_country_code=.."#.to_owned(),
            ],
        )?,
    );
    output = decode_escapes(
        &[[
            r#"\r Eurosports Region:\t\t\t"#.to_owned(),
            v.get(r#"Font_Green"#),
            slice_text(&v.get(r#"region"#), -2, None),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_francetv(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://geo-info.ftven.fr/ws/edgescape.json"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r France.tv:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"region"#,
        map_characters(
            &json_field(
                &[v.get(r#"tmpresult"#)].join(" "),
                &r#".reponse.geo_info.country_code"#.to_owned(),
            )?,
            &[r#"-d"#.to_owned(), r#"""#.to_owned()],
        )?,
    );
    if (v.get(r#"region"#) == r#"FR"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r France.tv:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes (Region: "#.to_owned(),
                v.get(r#"region"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r France.tv:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No   (Region: "#.to_owned(),
                v.get(r#"region"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r France.tv:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed (Network Connection)"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

async fn mediaunlocktest_hbo_nordic(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://api-discovery.hbo.eu/v1/discover/hbo?language=null&product=hbon"#
                .to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![header_pair(&r#"X-Client-Name: web"#.to_owned())?],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if ((v.get(r#"tmpresult"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r HBO Nordic:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        split_fields(
            &select_fields(
                &select_lines(
                    &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
                    &[r#"signupAllowed"#.to_owned()],
                )?,
                &[r#"{print $2}"#.to_owned()],
            )?,
            &[r#"-f1"#.to_owned(), r#"-d,"#.to_owned()],
        )?,
    );
    if (v.get(r#"result"#) == r#"true"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r HBO Nordic:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"false"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r HBO Nordic:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r HBO Nordic:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_hbo_spain(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://api-discovery.hbo.eu/v1/discover/hbo?language=null&product=hboe"#
                .to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![header_pair(&r#"X-Client-Name: web"#.to_owned())?],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if ((v.get(r#"tmpresult"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r HBO Spain:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        split_fields(
            &select_fields(
                &select_lines(
                    &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
                    &[r#"signupAllowed"#.to_owned()],
                )?,
                &[r#"{print $2}"#.to_owned()],
            )?,
            &[r#"-f1"#.to_owned(), r#"-d,"#.to_owned()],
        )?,
    );
    if (v.get(r#"result"#) == r#"true"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r HBO Spain:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"false"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r HBO Spain:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r HBO Spain:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_itvhub(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://simulcast.itv.com/playlist/itvonline/ITV"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r ITV Hub:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"404"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r ITV Hub:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r ITV Hub:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r ITV Hub:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Unexpected Result: "#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_megogotv(ctx: &mut Context) -> Result<String> {
    let mut request = Request::get(
        "https://ctx.playfamily.ru/screenapi/v4/preparepurchase/web/1?elementId=0b974dc3-d4c5-4291-9df5-81a8132f67c5&elementAlias=51459024&elementType=GAME&withUpgradeSubscriptionReturnAmount=true&forceSvod=true&includeProductsForUpsale=false",
    );
    request.timeout = 10;
    request
        .headers
        .push(["X-SCRAPI-CLIENT-TS".into(), timestamp(true)]);
    let body = ctx.request(request).await?;
    let result = match json_field(&body, ".status").as_deref() {
        Ok("0") if !ctx.denied => "Yes",
        _ => "Unknown",
    };
    Ok(format!("Megogo TV: {result}"))
}

async fn mediaunlocktest_molotov(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://fapi.molotov.tv/v1/open-europe/is-france"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Molotov:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        "?",
        if (select_lines(
            &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
            &[r#"false"#.to_owned()],
        )?)
        .is_empty()
        {
            "1"
        } else {
            "0"
        }
        .to_owned(),
    );
    if (number(&v.get(r#"?"#)) == number(&r#"0"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Molotov:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        "?",
        if (select_lines(
            &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
            &[r#"true"#.to_owned()],
        )?)
        .is_empty()
        {
            "1"
        } else {
            "0"
        }
        .to_owned(),
    );
    if (number(&v.get(r#"?"#)) == number(&r#"0"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Molotov:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Molotov:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_movistarplus(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://contratar.movistarplus.es/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: false,
            headers_only: true,
            include_headers: false,
            fail_status: false,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Movistar+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Movistar+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Movistar+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Movistar+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Movistar+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_npo_start_plus(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmptoken"#,
        ctx.request(Request {
            url: r#"https://npo.nl/start/api/domain/player-token?productId=BV_101410466"#
                .to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmptoken"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r NPO Start Plus:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"token"#,
        map_characters(
            &json_field(&[v.get(r#"tmptoken"#)].join(" "), &r#".token"#.to_owned())?,
            &[r#"-d"#.to_owned(), r#"""#.to_owned()],
        )?,
    );
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://prod.npoplayer.nl/stream-link"#.to_owned(),
            method: r#"POST"#.to_owned(),
            headers: vec![
                [r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)],
                header_pair(&[r#"Authorization:"#.to_owned(), v.get(r#"token"#)].concat())?,
            ],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r NPO Start Plus:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        json_field(&[v.get(r#"tmpresult"#)].join(" "), &r#".status"#.to_owned())?,
    );
    if (v.get(r#"result"#) == r#"451"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r NPO Start Plus:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r NPO Start Plus:\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_raiplay(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"tmpresult"#, ctx.request(Request { url: r#"https://mediapolisvod.rai.it/relinker/relinkerServlet.htm?cont=VxXwi7UcqjApssSlashbjsAghviAeeqqEEqualeeqqEEqual&output=64"#.to_owned(), method: r#"GET"#.to_owned(), headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]], body: None, follow: false, headers_only: false, include_headers: false, fail_status: false, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if ((v.get(r#"tmpresult"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Rai Play:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"no_available"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Rai Play:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Rai Play:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_rakutentv(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://rakuten.tv"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{url_effective}\n"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Rakuten TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"waitforit"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Rakuten TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Rakuten TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Rakuten TV:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed (Network Connection)"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

async fn mediaunlocktest_salto(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://geo.salto.fr/v1/geoInfo/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Salto:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"CountryCode"#,
        split_fields(
            &select_lines(
                &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
                &[r#"country_code"#.to_owned()],
            )?,
            &[r#"-f4"#.to_owned(), r#"-d""#.to_owned()],
        )?,
    );
    v.set(
        r#"AllowedCode"#,
        r#"FR,GP,MQ,GF,RE,YT,PM,BL,MF,WF,PF,NC"#.to_owned(),
    );
    v.set(
        "?",
        if (select_lines(
            &[v.get(r#"AllowedCode"#)].join(" "),
            &[v.get(r#"CountryCode"#)],
        )?)
        .is_empty()
        {
            "1"
        } else {
            "0"
        }
        .to_owned(),
    );
    if (number(&v.get(r#"?"#)) == number(&r#"0"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Salto:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Salto:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_sky_ch(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://sky.ch/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: true,
            include_headers: false,
            fail_status: false,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{url_effective}\n"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r SKY CH:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"out-of-country"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r SKY CH:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r SKY CH:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r SKY CH:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

async fn mediaunlocktest_skygo(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"tmpresult"#, ctx.request(Request { url: r#"https://skyid.sky.com/authorise/skygo?response_type=token&client_id=sky&appearance=compact&redirect_uri=skygo://auth"#.to_owned(), method: r#"GET"#.to_owned(), headers: vec![], body: None, follow: true, headers_only: false, include_headers: false, fail_status: false, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if ((v.get(r#"tmpresult"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Sky Go:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"You don't have permission to access"#.to_owned()],
        )?,
    );
    if ((v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Sky Go:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Sky Go:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_skyshowtime(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"tmpresult"#, ctx.request(Request { url: r#"https://www.skyshowtime.com/"#.to_owned(), method: r#"GET"#.to_owned(), headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)], header_pair(&r#"accept: text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9"#.to_owned())?], body: None, follow: false, headers_only: false, include_headers: true, fail_status: true, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r SkyShowTime:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result1"#,
        select_fields(
            &take_lines(
                &select_lines(
                    &[v.get(r#"tmpresult"#)].join(" "),
                    &[r#"-i"#.to_owned(), r#"^location:"#.to_owned()],
                )?,
                &[r#"-1"#.to_owned()],
            )?,
            &[r#"{print $2}"#.to_owned()],
        )?,
    );
    if (matches_pattern(&v.get(r#"result1"#), &r#"*where-can-i-stream*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r SkyShowTime:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        v.set(
            r#"region1"#,
            map_characters(
                &replace_text(
                    &[v.get(r#"result1"#)].join(" "),
                    &[
                        r#"s#https://www.skyshowtime.com/\([0-9a-zA-Z][0-9a-zA-Z]\)?\r#\1#i"#
                            .to_owned(),
                    ],
                )?,
                &[r#"[:lower:]"#.to_owned(), r#"[:upper:]"#.to_owned()],
            )?,
        );
        output = decode_escapes(
            &[[
                r#"\r SkyShowTime:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes (Region: "#.to_owned(),
                v.get(r#"region1"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_viaplay(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result1"#,
        ctx.request(Request {
            url: r#"https://viaplay.pl/package?recommended=viaplay"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{url_effective}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    v.set(
        r#"result2"#,
        ctx.request(Request {
            url: r#"https://viaplay.com/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{redirect_url}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"result1"#), &r#"curl*"#.to_owned()))
        || (matches_pattern(&v.get(r#"result2"#), &r#"curl*"#.to_owned()))
    {
        output = decode_escapes(
            &[[
                r#"\r Viaplay:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (matches_pattern(&v.get(r#"result1"#), &r#"*region-blocked*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Viaplay:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        v.set(
            r#"region"#,
            select_fields(
                &[v.get(r#"result2"#)].join(" "),
                &[r#"-F/"#.to_owned(), r#"{print $4}"#.to_owned()],
            )?,
        );
        output = decode_escapes(
            &[[
                r#"\r Viaplay:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes (Region: "#.to_owned(),
                v.get(r#"region"#).to_uppercase(),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Viaplay:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_videoland(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"tmpresult"#, ctx.request(Request { url: r#"https://api.videoland.com/subscribe/videoland-account/graphql"#.to_owned(), method: r#"POST"#.to_owned(), headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)], header_pair(&r#"content-type: application/json"#.to_owned())?], body: Some([r#"{"operationName":"IsOnboardingGeoBlocked","variables":{},"query":"query IsOnboardingGeoBlocked {\n isOnboardingGeoBlocked\n}\n"}"#.to_owned()].join("&")), follow: false, headers_only: false, include_headers: false, fail_status: false, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r videoland:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        json_field(
            &[v.get(r#"tmpresult"#)].join(" "),
            &r#".data.isOnboardingGeoBlocked"#.to_owned(),
        )?,
    );
    if (v.get(r#"result"#) == r#"false"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r videoland:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r videoland:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_zdf(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://ssl.zdf.de/geo/de/geo.txt/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Dalvik"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r ZDF: \t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"404"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r ZDF: \t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r ZDF: \t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r ZDF: \t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Unexpected Result: "#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_abciview(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"tmpresult"#, ctx.request(Request { url: r#"https://api.iview.abc.net.au/v2/show/abc-kids-live-stream/video/LS1604H001S00?embed=highlightVideo,selectedSeries"#.to_owned(), method: r#"GET"#.to_owned(), headers: vec![], body: None, follow: false, headers_only: false, include_headers: false, fail_status: false, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if ((v.get(r#"tmpresult"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r ABC iView:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"unavailable outside Australia"#.to_owned()],
        )?,
    );
    if ((v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r ABC iView:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r ABC iView:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_binge(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://auth.streamotion.com.au"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Binge:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"200"#.to_owned())
        || (v.get(r#"result"#) == r#"302"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r Binge:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Binge:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Binge:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Unexpected Result: "#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_channel10(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://e410fasadvz.global.ssl.fastly.net/geo"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r Channel 10:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Channel 10:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        split_fields(
            &select_fields(
                &select_lines(
                    &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
                    &[r#"allow"#.to_owned()],
                )?,
                &[r#"{print $2}"#.to_owned()],
            )?,
            &[r#"-f1"#.to_owned(), r#"-d,"#.to_owned()],
        )?,
    );
    if (v.get(r#"result"#) == r#"false"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Channel 10:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"true"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Channel 10:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Channel 10:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed (Network Connection)"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

async fn mediaunlocktest_channel9(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        select_lines(
            &ctx.request(Request {
                url: r#"https://login.nine.com.au"#.to_owned(),
                method: r#"GET"#.to_owned(),
                headers: vec![],
                body: None,
                follow: true,
                headers_only: false,
                include_headers: false,
                fail_status: false,
                discard_body: true,
                timeout: 10,
                writeout: r#"%{url_effective}\n"#.to_owned(),
                cookie_store: false,
                cookie_load: false,
                retries: 0,
                tls13: false,
            })
            .await?,
            &[r#"geoblock"#.to_owned()],
        )?,
    );
    if (matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Channel 9:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Channel 9:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Channel 9:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Channel 9:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed (Network Connection)"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

async fn mediaunlocktest_docplay(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        select_lines(
            &ctx.request(Request {
                url: r#"https://www.docplay.com/subscribe"#.to_owned(),
                method: r#"GET"#.to_owned(),
                headers: vec![],
                body: None,
                follow: true,
                headers_only: false,
                include_headers: false,
                fail_status: false,
                discard_body: true,
                timeout: 10,
                writeout: r#"%{url_effective}\n"#.to_owned(),
                cookie_store: false,
                cookie_load: false,
                retries: 0,
                tls13: false,
            })
            .await?,
            &[r#"geoblocked"#.to_owned()],
        )?,
    );
    if (matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Docplay:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        v.set(r#"isKayoSportsOK"#, r#"2"#.to_owned());
        return Ok(output);
    } else if (!(v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Docplay:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        v.set(r#"isKayoSportsOK"#, r#"0"#.to_owned());
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Docplay:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        v.set(r#"isKayoSportsOK"#, r#"1"#.to_owned());
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Docplay:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed (Network Connection)"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    v.set(r#"isKayoSportsOK"#, r#"2"#.to_owned());
    return Ok(output);
    Ok(output)
}

async fn mediaunlocktest_kayosports(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://auth.streamotion.com.au"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Kayo Sports:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"200"#.to_owned())
        || (v.get(r#"result"#) == r#"302"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r Kayo Sports:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Kayo Sports:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Kayo Sports:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Unexpected Result: "#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_sbsondemand(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://www.sbs.com.au/api/v3/network?context=odwebsite"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r SBS on Demand:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        split_fields(
            &select_lines(
                &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
                &[r#"country_code"#.to_owned()],
            )?,
            &[r#"-f4"#.to_owned(), r#"-d""#.to_owned()],
        )?,
    );
    if (v.get(r#"result"#) == r#"AU"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r SBS on Demand:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r SBS on Demand:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r SBS on Demand:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed "#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

async fn mediaunlocktest_skygonz(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://linear-s.stream.skyone.co.nz/sky-sport-1.mpd"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r SkyGo NZ:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r SkyGo NZ:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r SkyGo NZ:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r SkyGo NZ:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_stan(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://api.stan.com.au/login/v1/sessions/web/account"#.to_owned(),
            method: r#"POST"#.to_owned(),
            headers: vec![],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if ((v.get(r#"tmpresult"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Stan:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"VPNDetected"#.to_owned()],
        )?,
    );
    if ((v.get(r#"result"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Stan:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Stan:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_threenow(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://bravo-livestream.fullscreen.nz/index.m3u8"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r ThreeNow:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r ThreeNow:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r ThreeNow:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r ThreeNow:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Unexpected Result: "#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_afreeca(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://vod.afreecatv.com/player/97464151"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r Afreeca TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Afreeca TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result1"#,
        select_lines(
            &[v.get(r#"tmpresult"#)].join(" "),
            &[r#"document.location.href='https://vod.afreecatv.com'"#.to_owned()],
        )?,
    );
    if ((v.get(r#"result1"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Afreeca TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r Afreeca TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_coupangplay(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://www.coupangplay.com/"#.to_owned(),
            method: r#"HEAD"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: false,
            headers_only: true,
            include_headers: false,
            fail_status: true,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r Coupang Play:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Coupang Play:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result1"#,
        select_fields(
            &select_lines(
                &[v.get(r#"tmpresult"#)].join(" "),
                &[r#"Location"#.to_owned(), r#"-i"#.to_owned()],
            )?,
            &[r#"{print $2}"#.to_owned()],
        )?,
    );
    if (matches_pattern(&v.get(r#"result1"#), &r#"*/not-available*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Coupang Play:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r Coupang Play:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_kbsdomestic(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"tmpresult"#, ctx.request(Request { url: r#"https://vod.kbs.co.kr/index.html?source=episode&sname=vod&stype=vod&program_code=T2022-0690&program_id=PS-2022164275-01-000&broadcast_complete_yn=N&local_station_code=00&section_code=03"#.to_owned(), method: r#"GET"#.to_owned(), headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]], body: None, follow: true, headers_only: false, include_headers: false, fail_status: true, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r KBS Domestic:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r KBS Domestic:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result1"#,
        select_lines(
            &select_lines(&[v.get(r#"tmpresult"#)].join(" "), &[r#"ipck"#.to_owned()])?,
            &[r#"Domestic\\": true"#.to_owned()],
        )?,
    );
    if ((v.get(r#"result1"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r KBS Domestic:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r KBS Domestic:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_pandatv(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result1"#,
        ctx.request(Request {
            url: r#"https://api.pandalive.co.kr/v1/live/play"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result1"#) == r#"000"#.to_owned()) && (v.get(r#"1"#) == r#"6"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r PandaTV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result1"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r PandaTV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (v.get(r#"result1"#) == r#"400"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r PandaTV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result1"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r PandaTV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r PandaTV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Unknown (Code: "#.to_owned(),
                v.get(r#"result1"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_aisplay(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"result"#, ctx.request(Request { url: r#"https://49-231-37-237-rewriter.ais-vidnt.com/ais/play/origin/VOD/playlist/ais-yMzNH1-bGUxc/index.m3u8"#.to_owned(), method: r#"HEAD"#.to_owned(), headers: vec![], body: None, follow: true, headers_only: true, include_headers: false, fail_status: false, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r AIS Play:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r AIS Play:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result1"#,
        select_fields(
            &select_lines(
                &[v.get(r#"result"#)].join(" "),
                &[r#"X-Geo-Protection-System-Status"#.to_owned()],
            )?,
            &[r#"{print $2}"#.to_owned()],
        )?,
    );
    if (matches_pattern(&v.get(r#"result1"#), &r#"*ALLOW*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r AIS Play:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"result1"#), &r#"*BLOCK*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r AIS Play:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r AIS Play:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_bein_sports(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"result"#, ctx.request(Request { url: r#"https://d6m3sfa7e58z5.cloudfront.net/out/v1/3b0660e05eed4d769521eb0275aab3ab/index.mpd"#.to_owned(), method: r#"GET"#.to_owned(), headers: vec![], body: None, follow: true, headers_only: false, include_headers: false, fail_status: true, discard_body: true, timeout: 10, writeout: r#"%{http_code}"#.to_owned(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r beIN Sports:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r beIN Sports:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r beIN Sports:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    } else {
        output = decode_escapes(
            &[[
                r#"\r beIN Sports:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Unexpected Result: "#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_cliptv(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://cliptv.vn/the-forgiven-2022,lvDoN0JrP/Jx52YAKL2v"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"result"#), &r#"*curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r Clip TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"result"#), &r#"*curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Clip TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (matches_pattern(
        &v.get(r#"result"#),
        &r#"*Sorry, this video is not available in your country.*"#.to_owned(),
    )) {
        output = decode_escapes(
            &[[
                r#"\r Clip TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Clip TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Clip TV:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_galaxyplay(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://api.glxplay.io/account/device/new"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"result"#), &r#"*curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r Galaxy Play:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"result"#), &r#"*curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Galaxy Play:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (matches_pattern(&v.get(r#"result"#), &r#"*495*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Galaxy Play:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Galaxy Play:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Galaxy Play:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_jiocinema(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://content-jiovoot.voot.com/psapi/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r Jio Cinema:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Jio Cinema:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"code"#,
        json_field(&[v.get(r#"tmpresult"#)].join(" "), &r#".code"#.to_owned())?,
    );
    if (v.get(r#"code"#) == r#"474"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Jio Cinema:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Jio Cinema:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Jio Cinema:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_mewatch(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(r#"result"#, ctx.request(Request { url: r#"https://cdn.mewatch.sg/api/items/362414/videos?delivery=stream%2Cprogressive&ff=idp%2Cldp%2Crpt%2Ccd&lang=en&resolution=External&segments=all"#.to_owned(), method: r#"GET"#.to_owned(), headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]], body: None, follow: true, headers_only: false, include_headers: false, fail_status: false, discard_body: false, timeout: 10, writeout: String::new(), cookie_store: false, cookie_load: false, retries: 0, tls13: false }).await?);
    if (matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r meWATCH:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r meWATCH:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (matches_pattern(
        &v.get(r#"result"#),
        &r#"*location that is not permitted*"#.to_owned(),
    )) {
        output = decode_escapes(
            &[[
                r#"\r meWATCH:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r meWATCH:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r meWATCH:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_mxplayer(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://www.mxplayer.in/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"result"#), &r#"*curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r MXPlayer:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"result"#), &r#"*curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r MXPlayer:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (matches_pattern(
        &v.get(r#"result"#),
        &r#"*We are currently not available in your region*"#.to_owned(),
    )) {
        output = decode_escapes(
            &[[
                r#"\r MXPlayer:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r MXPlayer:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r MXPlayer:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_mytv(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://webapi.mytv.vn/api/v1/movie/138546/play?"#.to_owned(),
            method: r#"POST"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: Some([r#"partition=1"#.to_owned()].join("&")),
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"result"#), &r#"*curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r MYTV:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"result"#), &r#"*curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r MYTV:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result1"#,
        map_characters(
            &select_fields(
                &select_lines(
                    &json_field(&[v.get(r#"result"#)].join(" "), &".")?,
                    &[r#"result"#.to_owned()],
                )?,
                &[r#"{print $2}"#.to_owned()],
            )?,
            &[r#"-d"#.to_owned(), r#","#.to_owned()],
        )?,
    );
    if (v.get(r#"result1"#) == r#"103"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r MYTV:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r MYTV:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r MYTV:\t\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_sonyliv(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://www.sonyliv.com/signin"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r SonyLiv:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r SonyLiv:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"country"#,
        select_lines(
            &select_lines(
                &[v.get(r#"tmpresult"#)].join(" "),
                &[r#"country_code"#.to_owned()],
            )?,
            &[r#"-Eo"#.to_owned(), r#"country_code:..."#.to_owned()],
        )?,
    );
    if (matches_pattern(&v.get(r#"country"#), &r#"*IN"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r SonyLiv:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes (Region:"#.to_owned(),
                trim_prefix_pattern(&v.get(r#"country"#), &r#"*""#.to_owned()),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r SonyLiv:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No  (Region:"#.to_owned(),
                trim_prefix_pattern(&v.get(r#"country"#), &r#"*""#.to_owned()),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r SonyLiv:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_sooka(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url:
                r#"https://app-expmanager-proxy.sooka.my/prod/api/v1/enveu_prod/screen?screenId=0"#
                    .to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Sooka:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Sooka:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Sooka:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Sooka:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Unexpected Result: "#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_starhubtvplus(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://ucdn.starhubgo.com/bpk-tv/HubSensasiHD/output/manifest.mpd"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r Starhub TV+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Starhub TV+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Starhub TV+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Starhub TV+:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Starhub TV+:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_tataplay(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://watch.tataplay.com/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"000"#.to_owned()) && (v.get(r#"1"#) == r#"6"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Tata Play:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Tata Play:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Tata Play:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Tata Play:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Tata Play:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_vidio(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://geo-id-media-001-vidio-com.akamaized.net/2x2.png"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"000"#.to_owned()) && (v.get(r#"1"#) == r#"6"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Vidio:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Vidio:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Vidio:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Vidio:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Vidio:\t\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_vtvcab(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"token"#,
        map_characters(
            &select_lines(
                &select_lines(
                    &ctx.request(Request {
                        url: r#"https://www.vtvcab.vn/"#.to_owned(),
                        method: r#"HEAD"#.to_owned(),
                        headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
                        body: None,
                        follow: true,
                        headers_only: true,
                        include_headers: false,
                        fail_status: false,
                        discard_body: false,
                        timeout: 10,
                        writeout: String::new(),
                        cookie_store: false,
                        cookie_load: false,
                        retries: 0,
                        tls13: false,
                    })
                    .await?,
                    &[r#"token"#.to_owned()],
                )?,
                &[r#"-Eo"#.to_owned(), r#"token=([^;]*)"#.to_owned()],
            )?,
            &[r#"-d"#.to_owned(), r#"token="#.to_owned()],
        )?,
    );
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://apigwon.gviet.vn/sdp-vod/api/v1/source"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![
                [r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)],
                header_pair(&[r#"authorization: "#.to_owned(), v.get(r#"token"#)].concat())?,
            ],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r VTVcab:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r VTVcab:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = [
        v.get(r#"result"#),
        r#">"#.to_owned(),
        r#"debug.logs"#.to_owned(),
    ]
    .join(" ");
    if (matches_pattern(&v.get(r#"result"#), &r#"*4006*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r VTVcab:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r VTVcab:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r VTVcab:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_zee5(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://www.zee5.com/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![
                header_pair(&r#"Upgrade-Insecure-Requests: 1"#.to_owned())?,
                [r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)],
            ],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: true,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: r#"_TAG_%{http_code}_TAG_"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    v.set(
        r#"httpCode"#,
        select_fields(
            &select_lines(&[v.get(r#"tmpresult"#)].join(" "), &[r#"_TAG_"#.to_owned()])?,
            &[r#"-F_TAG_"#.to_owned(), r#"{print $2}"#.to_owned()],
        )?,
    );
    if (v.get(r#"httpCode"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Zee5:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"region"#,
        take_lines(
            &select_lines(
                &[v.get(r#"tmpresult"#)].join(" "),
                &[r#"-woP"#.to_owned(), r#"country=\K[A-Z]{2}"#.to_owned()],
            )?,
            &[r#"-n"#.to_owned(), r#"1"#.to_owned()],
        )?,
    );
    if (!(v.get(r#"region"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r Zee5:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes (Region: "#.to_owned(),
                v.get(r#"region"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Zee5:\t\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed (Error: Unknown)"#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    Ok(output)
}

async fn mediaunlocktest_dstv(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://now.dstv.com/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r DStv:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r DStv:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"451"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r DStv:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r DStv:\t\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Unexpected Result: "#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_beinconnect(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://proxies.bein-mena-production.eu-west-2.tuc.red/proxy/availableOffers"#
                .to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"000"#.to_owned()) && (v.get(r#"1"#) == r#"6"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Bein Sports Connect:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Bein Sports Connect:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"500"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Bein Sports Connect:\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"451"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Bein Sports Connect:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r Bein Sports Connect:\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Unexpected Result: "#.to_owned(),
                v.get(r#"result"#),
                r#")"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_molatv(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://mola.tv/api/v2/videos/geoguard/check/vd30491025"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned()))
        && (v.get(r#"1"#) == r#"6"#.to_owned())
    {
        output = decode_escapes(
            &[[
                r#"\r Mola TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"IPv6 Not Support"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (matches_pattern(&v.get(r#"tmpresult"#), &r#"curl*"#.to_owned())) {
        output = decode_escapes(
            &[[
                r#"\r Mola TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        select_fields(
            &select_lines(
                &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
                &[r#"isAllowed"#.to_owned()],
            )?,
            &[r#"{print $2}"#.to_owned()],
        )?,
    );
    if (v.get(r#"result"#) == r#"true"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Mola TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"false"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r Mola TV:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    output = decode_escapes(
        &[[
            r#"\r Mola TV:\t\t\t\t"#.to_owned(),
            v.get(r#"Font_Red"#),
            r#"Failed "#.to_owned(),
            v.get(r#"Font_Suffix"#),
            r#"\n"#.to_owned(),
        ]
        .concat()]
        .join(" "),
    );
    return Ok(output);
    Ok(output)
}

async fn mediaunlocktest_cinemax(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://play.maxgo.com/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{http_code}"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (v.get(r#"result"#) == r#"000"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r CineMax Go:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"200"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r CineMax Go:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"403"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r CineMax Go:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    Ok(output)
}

async fn mediaunlocktest_hbonow(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"result"#,
        ctx.request(Request {
            url: r#"https://play.hbonow.com/"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![[r#"user-agent"#.to_owned(), v.get(r#"UA_Browser"#)]],
            body: None,
            follow: true,
            headers_only: false,
            include_headers: false,
            fail_status: true,
            discard_body: true,
            timeout: 10,
            writeout: r#"%{url_effective}\n"#.to_owned(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if (!matches_pattern(&v.get(r#"result"#), &r#"curl*"#.to_owned())) {
        if (v.get(r#"result"#) == r#"https://play.hbonow.com"#.to_owned())
            || (v.get(r#"result"#) == r#"https://play.hbonow.com/"#.to_owned())
        {
            output = decode_escapes(
                &[[
                    r#"\r HBO Now:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Green"#),
                    r#"Yes"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
        } else if (v.get(r#"result"#)
            == r#"http://hbogeo.cust.footprint.net/hbonow/geo.html"#.to_owned())
            || (v.get(r#"result"#) == r#"http://geocust.hbonow.com/hbonow/geo.html"#.to_owned())
        {
            output = decode_escapes(
                &[[
                    r#"\r HBO Now:\t\t\t\t"#.to_owned(),
                    v.get(r#"Font_Red"#),
                    r#"No"#.to_owned(),
                    v.get(r#"Font_Suffix"#),
                    r#"\n"#.to_owned(),
                ]
                .concat()]
                .join(" "),
            );
        }
    } else {
        output = decode_escapes(
            &[[
                r#"\r HBO Now:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}

async fn mediaunlocktest_hbo_portugal(ctx: &mut Context) -> Result<String> {
    let mut v = Variables::new();
    let mut output = String::new();
    v.set(
        r#"tmpresult"#,
        ctx.request(Request {
            url: r#"https://api.ugw.hbogo.eu/v3.0/GeoCheck/json/PRT"#.to_owned(),
            method: r#"GET"#.to_owned(),
            headers: vec![],
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 10,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        })
        .await?,
    );
    if ((v.get(r#"tmpresult"#)).is_empty()) {
        output = decode_escapes(
            &[[
                r#"\r HBO Portugal:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed (Network Connection)"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    }
    v.set(
        r#"result"#,
        split_fields(
            &select_fields(
                &select_lines(
                    &json_field(&[v.get(r#"tmpresult"#)].join(" "), &".")?,
                    &[r#"allow"#.to_owned()],
                )?,
                &[r#"{print $2}"#.to_owned()],
            )?,
            &[r#"-f1"#.to_owned(), r#"-d,"#.to_owned()],
        )?,
    );
    if (v.get(r#"result"#) == r#"1"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r HBO Portugal:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Green"#),
                r#"Yes"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else if (v.get(r#"result"#) == r#"0"#.to_owned()) {
        output = decode_escapes(
            &[[
                r#"\r HBO Portugal:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"No"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
        return Ok(output);
    } else {
        output = decode_escapes(
            &[[
                r#"\r HBO Portugal:\t\t\t\t"#.to_owned(),
                v.get(r#"Font_Red"#),
                r#"Failed"#.to_owned(),
                v.get(r#"Font_Suffix"#),
                r#"\n"#.to_owned(),
            ]
            .concat()]
            .join(" "),
        );
    }
    Ok(output)
}
