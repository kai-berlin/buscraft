//! Tests for the KNX file import, using small synthetic files (and the person's sample
//! files in ../reference when they are present).

use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Write};
use std::path::Path;

use crate::knxapp::*;
use crate::knxfile::*;
use crate::knxproj::rtf_to_text;

#[test]
fn tests_and_text() {
    assert!(test_matches(&["1".into(), "2".into()], "2"));
    assert!(test_matches(&["!=0".into()], "5"));
    assert!(test_matches(&["<5".into()], "3"));
    assert!(!test_matches(&[">=5".into()], "3"));
    assert_eq!(clean_text("Kanal {{0:A}} Ausgang"), "Kanal A Ausgang");
    assert_eq!(rtf_to_text(r"{\rtf1\ansi{\fonttbl{\f0 Arial;}}\f0 Hallo\par Welt}"), "Hallo\nWelt");
}

const APP: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<KNX xmlns="http://knx.org/xml/project/23"><ManufacturerData><Manufacturer RefId="M-0001"><ApplicationPrograms>
<ApplicationProgram Id="M-0001_A-0001-10-ABCD" Name="Test switch" ApplicationVersion="18" DefaultLanguage="de-DE"><Static>
<ParameterTypes>
 <ParameterType Id="M-0001_A-0001-10-ABCD_PT-YN" Name="YN"><TypeRestriction Base="Value" SizeInBit="1">
   <Enumeration Id="M-0001_A-0001-10-ABCD_PT-YN_EN-0" Text="nein" Value="0"/><Enumeration Id="M-0001_A-0001-10-ABCD_PT-YN_EN-1" Text="ja" Value="1"/></TypeRestriction></ParameterType>
 <ParameterType Id="M-0001_A-0001-10-ABCD_PT-T" Name="T"><TypeNumber SizeInBit="8" Type="unsignedInt" minInclusive="0" maxInclusive="60"/></ParameterType>
</ParameterTypes>
<Parameters>
 <Parameter Id="M-0001_A-0001-10-ABCD_P-1" Name="Delay" ParameterType="M-0001_A-0001-10-ABCD_PT-YN" Text="Verzögerung" Value="0"/>
 <Parameter Id="M-0001_A-0001-10-ABCD_P-2" Name="Time" ParameterType="M-0001_A-0001-10-ABCD_PT-T" Text="Zeit" Value="5" SuffixText="s"/>
</Parameters>
<ParameterRefs>
 <ParameterRef Id="M-0001_A-0001-10-ABCD_P-1_R-1" RefId="M-0001_A-0001-10-ABCD_P-1"/>
 <ParameterRef Id="M-0001_A-0001-10-ABCD_P-2_R-2" RefId="M-0001_A-0001-10-ABCD_P-2"/>
</ParameterRefs>
<ComObjectTable>
 <ComObject Id="M-0001_A-0001-10-ABCD_O-0" Name="Switch" Text="Schalten" Number="0" FunctionText="Kanal A" ObjectSize="1 Bit" ReadFlag="Disabled" WriteFlag="Enabled" CommunicationFlag="Enabled" TransmitFlag="Disabled" UpdateFlag="Disabled" DatapointType="DPST-1-1"/>
 <ComObject Id="M-0001_A-0001-10-ABCD_O-1" Name="Status" Text="Status" Number="1" FunctionText="Kanal A" ObjectSize="1 Bit" ReadFlag="Enabled" WriteFlag="Disabled" CommunicationFlag="Enabled" TransmitFlag="Enabled" UpdateFlag="Disabled" DatapointType="DPST-1-1"/>
</ComObjectTable>
<ComObjectRefs>
 <ComObjectRef Id="M-0001_A-0001-10-ABCD_O-0_R-1" RefId="M-0001_A-0001-10-ABCD_O-0"/>
 <ComObjectRef Id="M-0001_A-0001-10-ABCD_O-1_R-2" RefId="M-0001_A-0001-10-ABCD_O-1"/>
</ComObjectRefs>
</Static><Dynamic><ChannelIndependentBlock>
 <ParameterBlock Id="M-0001_A-0001-10-ABCD_PB-1" Name="General" Text="Allgemein">
   <ParameterRefRef RefId="M-0001_A-0001-10-ABCD_P-1_R-1"/>
   <choose ParamRefId="M-0001_A-0001-10-ABCD_P-1_R-1">
     <when test="1"><ParameterRefRef RefId="M-0001_A-0001-10-ABCD_P-2_R-2"/><ComObjectRefRef RefId="M-0001_A-0001-10-ABCD_O-1_R-2"/></when>
   </choose>
 </ParameterBlock>
 <ComObjectRefRef RefId="M-0001_A-0001-10-ABCD_O-0_R-1"/>
</ChannelIndependentBlock></Dynamic>
</ApplicationProgram></ApplicationPrograms>
<Languages><Language Identifier="en-US"><TranslationUnit RefId="M-0001_A-0001-10-ABCD">
 <TranslationElement RefId="M-0001_A-0001-10-ABCD_P-1"><Translation AttributeName="Text" Text="Delay"/></TranslationElement>
 <TranslationElement RefId="M-0001_A-0001-10-ABCD_PT-YN_EN-1"><Translation AttributeName="Text" Text="yes"/></TranslationElement>
</TranslationUnit></Language></Languages>
</Manufacturer></ManufacturerData></KNX>"#;

const HW: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<KNX xmlns="http://knx.org/xml/project/23"><ManufacturerData><Manufacturer RefId="M-0001"><Hardware>
<Hardware Id="M-0001_H-1" Name="Switch" HasIndividualAddress="true" HasApplicationProgram="true">
 <Products><Product Id="M-0001_H-1_P-1" Text="Test switch actuator" OrderNumber="TS-1"/></Products>
 <Hardware2Programs><Hardware2Program Id="M-0001_H-1_HP-0001-10-ABCD" MediumTypes="MT-0"><ApplicationProgramRef RefId="M-0001_A-0001-10-ABCD"/></Hardware2Program></Hardware2Programs>
</Hardware></Hardware></Manufacturer></ManufacturerData></KNX>"#;

const CAT: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<KNX xmlns="http://knx.org/xml/project/23"><ManufacturerData><Manufacturer RefId="M-0001"><Catalog>
<CatalogSection Id="M-0001_CS-1" Name="Aktoren" Number="1"><CatalogItem Id="M-0001_CI-1" Name="Test switch actuator" ProductRefId="M-0001_H-1_P-1" Hardware2ProgramRefId="M-0001_H-1_HP-0001-10-ABCD"/></CatalogSection>
</Catalog></Manufacturer></ManufacturerData></KNX>"#;

const MASTER: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<KNX xmlns="http://knx.org/xml/project/23"><MasterData><DatapointTypes><DatapointType Id="DPT-1" Number="1" Text="1-bit"><DatapointSubtypes><DatapointSubtype Id="DPST-1-1" Number="1" Text="switch"/></DatapointSubtypes></DatapointType></DatapointTypes>
<Manufacturers><Manufacturer Id="M-0001" Name="Test Maker"/></Manufacturers></MasterData></KNX>"#;

const PROJ: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<KNX xmlns="http://knx.org/xml/project/23"><Project Id="P-0001"><Installations><Installation Name="">
<Topology><Area Id="P-0001-0_A-1" Name="Haus" Address="1"><Line Id="P-0001-0_L-1" Name="TP Linie" Address="1" MediumTypeRefId="MT-0"><Segment Id="P-0001-0_S-1" Number="0" MediumTypeRefId="MT-0">
 <DeviceInstance Id="P-0001-0_DI-1" Name="Licht Küche" ProductRefId="M-0001_H-1_P-1" Hardware2ProgramRefId="M-0001_H-1_HP-0001-10-ABCD" Address="7" IndividualAddressLoaded="true" ApplicationProgramLoaded="true" ParametersLoaded="true" CommunicationPartLoaded="false">
   <ParameterInstanceRefs><ParameterInstanceRef RefId="M-0001_A-0001-10-ABCD_P-1_R-1" Value="1"/></ParameterInstanceRefs>
   <ComObjectInstanceRefs><ComObjectInstanceRef RefId="O-0_R-1" Links="GA-1"/><ComObjectInstanceRef RefId="O-1_R-2" Links="GA-2"/></ComObjectInstanceRefs>
 </DeviceInstance>
</Segment></Line></Area></Topology>
<Locations><Space Id="P-0001-0_BP-1" Name="Haus" Type="Building"><Space Id="P-0001-0_BP-2" Name="EG" Type="Floor"><Space Id="P-0001-0_BP-3" Name="Küche" Type="Room"><DeviceInstanceRef RefId="P-0001-0_DI-1"/></Space></Space></Space></Locations>
<GroupAddresses><GroupRanges><GroupRange Id="P-0001-0_GR-1" Name="Licht" RangeStart="0" RangeEnd="2047"><GroupRange Id="P-0001-0_GR-2" Name="EG" RangeStart="0" RangeEnd="255">
 <GroupAddress Id="P-0001-0_GA-1" Name="Küche schalten" Address="1" DatapointType="DPST-1-1"/>
 <GroupAddress Id="P-0001-0_GA-2" Name="Küche Status" Address="2" DatapointType="DPST-1-1"/>
</GroupRange></GroupRange></GroupRanges></GroupAddresses>
</Installation></Installations></Project></KNX>"#;

const INFO: &str = r#"<?xml version="1.0" encoding="utf-8"?><KNX xmlns="http://knx.org/xml/project/23"><Project Id="P-0001"><ProjectInformation Name="Testhaus" GroupAddressStyle="ThreeLevel"/></Project></KNX>"#;

fn build_knxproj(password: Option<&str>) -> std::path::PathBuf {
    use zip::write::SimpleFileOptions;
    let mut inner = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let zpw = password.map(ets6_zip_password);
    let opts = match &zpw {
        Some(z) => SimpleFileOptions::default().with_aes_encryption(zip::AesMode::Aes256, z),
        None => SimpleFileOptions::default(),
    };
    inner.start_file("0.xml", opts).unwrap();
    inner.write_all(PROJ.as_bytes()).unwrap();
    inner.start_file("project.xml", opts).unwrap();
    inner.write_all(INFO.as_bytes()).unwrap();
    let inner = inner.finish().unwrap().into_inner();
    let path = std::env::temp_dir().join(format!("bps-test-{}.knxproj", crate::model::fresh_id()));
    let mut outer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let o = SimpleFileOptions::default();
    for (n, t) in [("knx_master.xml", MASTER), ("M-0001/Hardware.xml", HW), ("M-0001/Catalog.xml", CAT), ("M-0001/M-0001_A-0001-10-ABCD.xml", APP)] {
        outer.start_file(n, o).unwrap();
        outer.write_all(t.as_bytes()).unwrap();
    }
    outer.start_file("P-0001.zip", o).unwrap();
    outer.write_all(&inner).unwrap();
    outer.finish().unwrap();
    path
}

#[test]
fn application_visibility() {
    let app = parse_application(APP, "en-US", &HashMap::new()).unwrap();
    assert_eq!(app.version, "1.2");
    let mut vals = BTreeMap::new();
    let v = app.visible(&vals);
    assert_eq!(v.pages.len(), 1);
    assert_eq!(v.pages[0].items.len(), 1);
    assert_eq!(v.coms.len(), 1);
    assert_eq!(app.params["M-0001_A-0001-10-ABCD_P-1_R-1"].text, "Delay");
    vals.insert("M-0001_A-0001-10-ABCD_P-1_R-1".to_string(), "1".to_string());
    let v = app.visible(&vals);
    assert_eq!(v.pages[0].items.len(), 2);
    assert_eq!(v.coms.len(), 2);
    assert_eq!(app.params["M-0001_A-0001-10-ABCD_P-2_R-2"].unit, "s");
    match &app.types["M-0001_A-0001-10-ABCD_PT-YN"] {
        PType::Enum(e) => assert_eq!(e[1].1, "yes"),
        _ => panic!(),
    }
}

#[test]
fn protected_project_import() {
    let path = build_knxproj(Some("geheim"));
    let p = Progress::default();
    assert!(matches!(import(&path, None, false, "en-US", &p), Err(ImportErr::NeedPassword(_))));
    assert!(matches!(import(&path, Some("falsch"), false, "en-US", &p), Err(ImportErr::WrongPassword(_))));
    let out = import(&path, Some("geheim"), false, "en-US", &p).unwrap();
    let proj = out.project.unwrap();
    assert_eq!(proj.name, "Testhaus");
    assert_eq!(proj.devices.len(), 1);
    let d = &proj.devices[0];
    assert_eq!(d.address(), "1.1.7");
    assert_eq!(d.manufacturer, "Test Maker");
    assert_eq!(d.order_no, "TS-1");
    assert!(d.adr_ok && !d.grp_ok);
    assert_eq!(d.objects.len(), 2, "status object visible because parameter = 1");
    assert_eq!(d.objects[0].links.len(), 1);
    assert_eq!(proj.ga(d.objects[0].links[0]).unwrap().name, "Küche schalten");
    assert_eq!(proj.addresses.len(), 2);
    assert_eq!(proj.main_groups[0].name, "Licht");
    assert_eq!(proj.main_groups[0].middles[0].name, "EG");
    assert_eq!(proj.addresses[0].dpt, "1.001 switch");
    assert_eq!(proj.room_name(d.id), "Küche");
    assert_eq!(out.products.len(), 1);
    assert_eq!(out.products[0].category, "Aktoren");
    let out = import(&path, None, true, "en-US", &p).unwrap();
    assert!(out.project.is_none());
    assert_eq!(out.apps.len(), 1);
    let _ = std::fs::remove_file(path);
}

#[test]
fn unprotected_project_import() {
    let path = build_knxproj(None);
    let out = import(&path, None, false, "de-DE", &Progress::default()).unwrap();
    assert_eq!(out.project.unwrap().devices.len(), 1);
    let _ = std::fs::remove_file(path);
}

/// Reads the person's sample files when they are present (they are not part of the build).
#[test]
fn reference_samples() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("reference");
    let Ok(rd) = std::fs::read_dir(&dir) else { return };
    for e in rd.flatten() {
        let path = e.path();
        let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("").to_lowercase();
        if !["knxprod", "knxproj", "vd1", "vd4"].contains(&ext.as_str()) {
            continue;
        }
        let t = std::time::Instant::now();
        let r = import(&path, None, false, "en-US", &Progress::default());
        match (&r, ext.as_str()) {
            (Ok(o), _) => {
                println!("{}: {} products, {} apps, {} warnings {:?} ({:?})", path.display(), o.products.len(), o.apps.len(), o.warnings.len(), o.warnings.iter().take(3).collect::<Vec<_>>(), t.elapsed());
                assert!(!o.products.is_empty());
                for a in o.apps.iter().take(6) {
                    let v = a.visible(&BTreeMap::new());
                    println!("   {} {}: {} params, {} objects, {} visible pages, {} visible objects {:?}", a.name, a.version, a.params.len(), a.coms.len(), v.pages.len(), v.coms.len(), a.notes);
                }
                for p in o.products.iter().take(3) {
                    println!("   product: {} | {} | {} | {} | {}", p.manufacturer, p.name, p.order_no, p.category, p.app_program);
                }
            }
            (Err(ImportErr::NeedPassword(n)), "knxproj") => {
                println!("{}: needs password ({n})", path.display());
                let o = import(&path, None, true, "en-US", &Progress::default()).unwrap();
                println!("   products only: {} products, {} apps ({:?})", o.products.len(), o.apps.len(), t.elapsed());
                for p in o.products.iter().take(8) {
                    println!("   product: {} | {} | {} | {}", p.manufacturer, p.name, p.order_no, p.app_program);
                }
                assert!(!o.products.is_empty());
            }
            (Err(e), "vd1" | "vd4") => println!("{}: {e}", path.display()),
            (Err(e), _) => panic!("{}: {e}", path.display()),
        }
    }
}

const APP_RULES: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<KNX xmlns="http://knx.org/xml/project/23"><ManufacturerData><Manufacturer RefId="M-0001"><ApplicationPrograms>
<ApplicationProgram Id="M-0001_A-0002-10-0000" Name="Rules" ApplicationVersion="16" MaskVersion="MV-07B0" LoadProcedureStyle="MergedProcedure"><Static>
<Code><RelativeSegment Id="M-0001_A-0002-10-0000_RS-04-00000" Size="4" LoadStateMachine="4" Offset="0"><Data>AAAAAA==</Data></RelativeSegment></Code>
<ParameterTypes>
 <ParameterType Id="M-0001_A-0002-10-0000_PT-B" Name="B"><TypeNumber SizeInBit="8" Type="unsignedInt" minInclusive="0" maxInclusive="255"/></ParameterType>
</ParameterTypes>
<Parameters>
 <Parameter Id="M-0001_A-0002-10-0000_P-1" Name="Mode" ParameterType="M-0001_A-0002-10-0000_PT-B" Text="Mode" Value="0"><Memory CodeSegment="M-0001_A-0002-10-0000_RS-04-00000" Offset="0" BitOffset="0"/></Parameter>
 <Parameter Id="M-0001_A-0002-10-0000_P-2" Name="Enable" ParameterType="M-0001_A-0002-10-0000_PT-B" Text="" Access="None" Value="0"><Memory CodeSegment="M-0001_A-0002-10-0000_RS-04-00000" Offset="1" BitOffset="0"/></Parameter>
 <Parameter Id="M-0001_A-0002-10-0000_P-3" Name="Mask" ParameterType="M-0001_A-0002-10-0000_PT-B" Text="" Access="None" Value="0"><Memory CodeSegment="M-0001_A-0002-10-0000_RS-04-00000" Offset="2" BitOffset="0"/></Parameter>
</Parameters>
<ParameterRefs>
 <ParameterRef Id="M-0001_A-0002-10-0000_P-1_R-1" RefId="M-0001_A-0002-10-0000_P-1"/>
 <ParameterRef Id="M-0001_A-0002-10-0000_P-2_R-2" RefId="M-0001_A-0002-10-0000_P-2"/>
 <ParameterRef Id="M-0001_A-0002-10-0000_P-3_R-3" RefId="M-0001_A-0002-10-0000_P-3"/>
</ParameterRefs>
<ParameterCalculations>
 <ParameterCalculation Id="M-0001_A-0002-10-0000_PC-1" Language="JavaScript" Name="MaskCalc">
  <LRTransformation>Mask = 0; if (Mode == 2) { Mask = Mask + 4; } if (Enable == 1) { Mask += 1; }</LRTransformation>
  <LParameters><ParameterRefRef RefId="M-0001_A-0002-10-0000_P-1_R-1"/><ParameterRefRef RefId="M-0001_A-0002-10-0000_P-2_R-2" AliasName="Enable"/></LParameters>
  <RParameters><ParameterRefRef RefId="M-0001_A-0002-10-0000_P-3_R-3"/></RParameters>
 </ParameterCalculation>
</ParameterCalculations>
</Static><Dynamic><ChannelIndependentBlock>
 <ParameterBlock Id="M-0001_A-0002-10-0000_PB-1" Text="General">
   <ParameterRefRef RefId="M-0001_A-0002-10-0000_P-1_R-1"/>
   <choose ParamRefId="M-0001_A-0002-10-0000_P-1_R-1">
     <when test="2"><Assign TargetParamRefRef="M-0001_A-0002-10-0000_P-2_R-2" Value="1"/></when>
   </choose>
 </ParameterBlock>
</ChannelIndependentBlock></Dynamic>
</ApplicationProgram></ApplicationPrograms></Manufacturer></ManufacturerData></KNX>"#;

#[test]
fn assign_rules_and_calculations() {
    let app = parse_application(APP_RULES, "en-US", &HashMap::new()).unwrap();
    assert_eq!(app.format, 3);
    assert_eq!(app.calcs.len(), 1);
    assert_eq!(app.calcs[0].inputs[0].1, "Mode", "alias defaults to the parameter name");
    let mut stored = BTreeMap::new();
    let (v, notes) = app.effective_values(&stored);
    assert!(notes.is_empty());
    assert_eq!(v.get("M-0001_A-0002-10-0000_P-3_R-3").map(|s| s.as_str()), Some("0"));
    stored.insert("M-0001_A-0002-10-0000_P-1_R-1".to_string(), "2".to_string());
    let (v, _) = app.effective_values(&stored);
    assert_eq!(v["M-0001_A-0002-10-0000_P-2_R-2"], "1", "assigned");
    assert_eq!(v["M-0001_A-0002-10-0000_P-3_R-3"], "5", "calculated from Mode=2 and Enable=1");
    // The memory image gets all three values
    let li = app.load.clone().unwrap();
    let mut dev = crate::model::Project::new("t", crate::model::Medium::IP, crate::model::Medium::TP, crate::model::GaStyle::ThreeLevel);
    let id = dev.alloc_id();
    let d = crate::model::Device {
        id,
        area: 1,
        line: 1,
        number: 1,
        name: "x".into(),
        description: String::new(),
        comment: String::new(),
        manufacturer: String::new(),
        product: String::new(),
        order_no: String::new(),
        app_program: "x".into(),
        serial: String::new(),
        params: vec![],
        objects: vec![],
        adr_ok: true,
        prg_ok: true,
        par_ok: true,
        grp_ok: true,
        cfg_ok: true,
        knx_app: Some(app.id.clone()),
        knx_values: v,
    };
    let img = crate::knxload::build_images(&li, &d, &dev).unwrap();
    assert_eq!(&img.by_obj[&4][..3], &[2, 1, 5]);
    dev.devices.push(d);
}
/// Evaluates the automatic parameter rules of every application in the person's sample files.
#[test]
fn reference_rules() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("reference");
    let Ok(rd) = std::fs::read_dir(&dir) else { return };
    let mut failures = vec![];
    for e in rd.flatten() {
        let path = e.path();
        let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("").to_lowercase();
        if ext != "knxprod" && ext != "knxproj" {
            continue;
        }
        let Ok(o) = import(&path, None, true, "en-US", &Progress::default()) else { continue };
        for a in &o.apps {
            fn count(n: &[Node]) -> usize {
                n.iter()
                    .map(|x| match x {
                        Node::Assign { .. } => 1,
                        Node::Block { children, .. } | Node::Channel { children, .. } => count(children),
                        Node::Choose { whens, .. } => whens.iter().map(|w| count(&w.children)).sum(),
                        _ => 0,
                    })
                    .sum()
            }
            let (v, notes) = a.effective_values(&BTreeMap::new());
            let assigns = count(&a.dynamic);
            if assigns > 0 || !a.calcs.is_empty() {
                println!("{} {}: {} assign rules, {} calculations, {} values set {:?}", a.name, a.version, assigns, a.calcs.len(), v.len(), notes);
            }
            failures.extend(notes);
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}
/// Builds the download plan for the person's real device (project store + sample project file),
/// when both are present on this computer. Read-only.
#[test]
fn person_device_plan() {
    let Some(appdata) = std::env::var_os("APPDATA") else { return };
    let base = std::path::PathBuf::from(appdata);
    let store = ["BusCraft", "BusPlanStudio"].iter().map(|d| base.join(d).join("projects.json")).find(|p| p.exists()).unwrap_or_default();
    let proj_file = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("reference").join("Wall26.knxproj");
    let (Ok(s), true) = (std::fs::read_to_string(&store), proj_file.exists()) else { return };
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    let Some(projects) = v.get("projects").and_then(|p| p.as_array()) else { return };
    let out = import(&proj_file, None, true, "en-US", &Progress::default()).unwrap();
    for p in projects {
        let Ok(proj) = serde_json::from_value::<crate::model::Project>(p.clone()) else { continue };
        for d in proj.devices.iter().filter(|d| d.knx_app.is_some()) {
            let Some(app) = out.apps.iter().find(|a| Some(&a.id) == d.knx_app.as_ref()) else { continue };
            let Some(li) = app.load.as_ref() else { continue };
            let Some(mask) = out.masks.iter().find(|m| m.id == li.mask) else { continue };
            let t = std::time::Instant::now();
            let (eff, notes) = app.effective_values(&d.knx_values);
            let mut d2 = d.clone();
            d2.knx_values = eff;
            let plan = crate::knxload::build_plan(li, mask, &d2, &proj, crate::knxload::DlKind::Full, None);
            match plan {
                Ok(p) => println!("{} {} {} ({}): plan ok, {} steps, {} bytes, {} values set, {:?} {:?} in {:?}", proj.name, d.address(), d.name, app.name, p.steps.len(), p.total_bytes(), d.knx_values.len(), notes, p.warnings, t.elapsed()),
                Err(e) => println!("{} {} {} ({}): {e}", proj.name, d.address(), d.name, app.name),
            }
        }
    }
}
/// Memory written by the original tool, reconstructed from its telegram recording
/// (reference/Programnming1.1.28.xml): (address → byte).
fn ets_memory(path: &Path) -> std::collections::BTreeMap<u32, u8> {
    let text = std::fs::read_to_string(path).unwrap();
    let doc = roxmltree::Document::parse(&text).unwrap();
    let mut mem = std::collections::BTreeMap::new();
    for t in doc.descendants().filter(|n| n.has_tag_name("Telegram") && n.attribute("Service") == Some("L_Data.con")) {
        let raw = t.attribute("RawData").unwrap_or("");
        let b: Vec<u8> = (0..raw.len() / 2).map(|i| u8::from_str_radix(&raw[i * 2..i * 2 + 2], 16).unwrap()).collect();
        let Some(f) = crate::knx::Frame::from_cemi(&b) else { continue };
        if f.tpdu.len() < 4 || f.tpdu[0] & 0xC0 != 0x40 {
            continue;
        }
        let apci = (((f.tpdu[0] & 3) as u16) << 8) | f.tpdu[1] as u16;
        if apci & 0x3C0 == 0x280 {
            let n = (apci & 0x3f) as usize;
            let addr = u16::from_be_bytes([f.tpdu[2], f.tpdu[3]]) as u32;
            for i in 0..n.min(f.tpdu.len() - 4) {
                mem.insert(addr + i as u32, f.tpdu[4 + i]);
            }
        }
    }
    mem
}

/// Compares what this app would write to 1.1.28 with what the original tool wrote.
#[test]
fn compare_with_original_tool_trace() {
    let refdir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("reference");
    let trace = refdir.join("Programnming1.1.28.xml");
    let Some(appdata) = std::env::var_os("APPDATA") else { return };
    let base = std::path::PathBuf::from(appdata);
    let store = ["BusCraft", "BusPlanStudio"].iter().map(|d| base.join(d).join("projects.json")).find(|p| p.exists()).unwrap_or_default();
    let (true, Ok(s)) = (trace.exists(), std::fs::read_to_string(&store)) else { return };
    let ets = ets_memory(&trace);
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    let proj = v["projects"].as_array().unwrap().iter().filter_map(|p| serde_json::from_value::<crate::model::Project>(p.clone()).ok()).find(|p| p.name == "Wall26").unwrap();
    let dev28 = proj.devices.iter().find(|d| d.address() == "1.1.28").unwrap();
    // BPS_COMPARE_VALUES_FROM=1.1.26 uses another device's settings (with 1.1.28's links)
    let mut dev_owned = dev28.clone();
    if let Ok(src) = std::env::var("BPS_COMPARE_VALUES_FROM") {
        let other = proj.devices.iter().find(|d| d.address() == src).unwrap();
        dev_owned.knx_values = other.knx_values.clone();
    }
    let dev = &dev_owned;
    let out = import(&refdir.join("Wall26.knxproj"), None, true, "en-US", &Progress::default()).unwrap();
    let app = out.apps.iter().find(|a| Some(&a.id) == dev.knx_app.as_ref()).unwrap();
    let li = app.load.clone().unwrap();
    let (eff, _) = app.effective_values(&dev.knx_values);
    let mut d2 = dev.clone();
    d2.knx_values = eff.clone();
    let active = app.active(&dev.knx_values);
    let img = crate::knxload::build_images_with(&li, &d2, &proj, Some(&active)).unwrap();
    // Base addresses as reported by the device in the recording
    let parts = [(4u8, 0x3F6Eu32, "application"), (3, 0x3C92, "group object table"), (2, 0x36DC, "association table"), (1, 0x3400, "address table")];
    let mut report = vec![];
    for (obj, base, name) in parts {
        let ours = &img.by_obj[&obj];
        let theirs: Vec<Option<u8>> = (0..ours.len().max(1)).map(|i| ets.get(&(base + i as u32)).copied()).collect();
        let written = theirs.iter().filter(|x| x.is_some()).count();
        let diffs: Vec<usize> = (0..ours.len()).filter(|i| theirs[*i].is_some() && theirs[*i] != Some(ours[*i])).collect();
        report.push(format!("{name}: ours {} bytes, original wrote {written}, {} differ", ours.len(), diffs.len()));
        for i in diffs.iter().take(60) {
            // Which parameters live in this byte?
            let who: Vec<String> = li
                .mem
                .iter()
                .filter(|(_, m)| obj == 4 && m.bit / 8 <= *i as u32 && (*i as u32) <= (m.bit + m.bits - 1) / 8)
                .map(|(p, m)| format!("{} (bit {} len {}) eff={:?}", p.trim_start_matches(&format!("{}_", app.id)), m.bit, m.bits, eff.iter().find(|(r, _)| li.ref_param.get(*r) == Some(p)).map(|x| x.1)))
                .collect();
            report.push(format!("  +{i:04X}: ours {:02X} original {:02X}  {}", ours[*i], theirs[*i].unwrap(), who.join("; ")));
        }
    }
    let text = report.join("\n");
    std::fs::write(Path::new(env!("CARGO_MANIFEST_DIR")).join("shots").join("compare-1.1.28.txt"), &text).unwrap();
    println!("{text}");
}