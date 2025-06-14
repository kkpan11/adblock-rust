//! Contains representations of data from the adblocking engine in a
//! forwards-and-backwards-compatible format, as well as utilities for converting these to and from
//! the actual `Engine` components.
//!
//! Any new fields should be added to the _end_ of both `SerializeFormat` and `DeserializeFormat`.

use std::collections::{HashMap, HashSet};

use rmp_serde as rmps;
use serde::{Deserialize, Serialize};

use crate::blocker::Blocker;
use crate::cosmetic_filter_cache::{CosmeticFilterCache, HostnameRuleDb, ProceduralOrActionFilter};
use crate::filters::fb_network::flat::fb;
use crate::filters::network::{NetworkFilter, NetworkFilterMaskHelper};
use crate::network_filter_list::NetworkFilterList;
use crate::utils::Hash;

use super::utils::{stabilize_hashmap_serialization, stabilize_hashset_serialization};
use super::{DeserializationError, SerializationError};

/// Each variant describes a single rule that is specific to a particular hostname.
#[derive(Clone, Debug, Deserialize, Serialize)]
enum LegacySpecificFilterType {
    Hide(String),
    Unhide(String),
    Style(String, String),
    UnhideStyle(String, String),
    ScriptInject(String),
    UnhideScriptInject(String),
}

#[derive(Deserialize, Serialize, Default)]
pub(crate) struct LegacyHostnameRuleDb {
    #[serde(serialize_with = "stabilize_hashmap_serialization")]
    db: HashMap<Hash, Vec<LegacySpecificFilterType>>,
}

impl From<&HostnameRuleDb> for LegacyHostnameRuleDb {
    fn from(v: &HostnameRuleDb) -> Self {
        let mut db = HashMap::<Hash, Vec<LegacySpecificFilterType>>::new();
        for (hash, bin) in v.hide.0.iter() {
            for f in bin {
                db.entry(*hash)
                    .and_modify(|v| v.push(LegacySpecificFilterType::Hide(f.to_owned())))
                    .or_insert_with(|| vec![LegacySpecificFilterType::Hide(f.to_owned())]);
            }
        }
        for (hash, bin) in v.unhide.0.iter() {
            for f in bin {
                db.entry(*hash)
                    .and_modify(|v| v.push(LegacySpecificFilterType::Unhide(f.to_owned())))
                    .or_insert_with(|| vec![LegacySpecificFilterType::Unhide(f.to_owned())]);
            }
        }
        for (hash, bin) in v.inject_script.0.iter() {
            for (f, _mask) in bin {
                db.entry(*hash)
                    .and_modify(|v| v.push(LegacySpecificFilterType::ScriptInject(f.to_owned())))
                    .or_insert_with(|| vec![LegacySpecificFilterType::ScriptInject(f.to_owned())]);
            }
        }
        for (hash, bin) in v.uninject_script.0.iter() {
            for f in bin {
                db.entry(*hash)
                    .and_modify(|v| {
                        v.push(LegacySpecificFilterType::UnhideScriptInject(f.to_owned()))
                    })
                    .or_insert_with(|| {
                        vec![LegacySpecificFilterType::UnhideScriptInject(f.to_owned())]
                    });
            }
        }
        for (hash, bin) in v.procedural_action.0.iter() {
            for f in bin {
                match serde_json::from_str::<ProceduralOrActionFilter>(f) {
                    Ok(f) => {
                        if let Some((selector, style)) = f.as_css() {
                            db.entry(*hash)
                                .and_modify(|v| {
                                    v.push(LegacySpecificFilterType::Style(
                                        selector.clone(),
                                        style.clone(),
                                    ))
                                })
                                .or_insert_with(|| {
                                    vec![LegacySpecificFilterType::Style(selector, style)]
                                });
                        }
                    }
                    _ => (),
                }
            }
        }
        for (hash, bin) in v.procedural_action_exception.0.iter() {
            for f in bin {
                match serde_json::from_str::<ProceduralOrActionFilter>(f) {
                    Ok(f) => {
                        if let Some((selector, style)) = f.as_css() {
                            db.entry(*hash)
                                .and_modify(|v| {
                                    v.push(LegacySpecificFilterType::UnhideStyle(
                                        selector.to_owned(),
                                        style.to_owned(),
                                    ))
                                })
                                .or_insert_with(|| {
                                    vec![LegacySpecificFilterType::UnhideStyle(
                                        selector.to_owned(),
                                        style.to_owned(),
                                    )]
                                });
                        }
                    }
                    _ => (),
                }
            }
        }
        LegacyHostnameRuleDb { db }
    }
}

impl Into<HostnameRuleDb> for LegacyHostnameRuleDb {
    fn into(self) -> HostnameRuleDb {
        use crate::cosmetic_filter_cache::HostnameFilterBin;

        let mut hide = HostnameFilterBin::default();
        let mut unhide = HostnameFilterBin::default();
        let mut procedural_action = HostnameFilterBin::default();
        let mut procedural_action_exception = HostnameFilterBin::default();
        let mut inject_script = HostnameFilterBin::default();
        let mut uninject_script = HostnameFilterBin::default();

        for (hash, bin) in self.db.into_iter() {
            for rule in bin.into_iter() {
                match rule {
                    LegacySpecificFilterType::Hide(s) => hide.insert(&hash, s),
                    LegacySpecificFilterType::Unhide(s) => unhide.insert(&hash, s),
                    LegacySpecificFilterType::Style(s, st) => procedural_action
                        .insert_procedural_action_filter(
                            &hash,
                            &ProceduralOrActionFilter::from_css(s, st),
                        ),
                    LegacySpecificFilterType::UnhideStyle(s, st) => procedural_action_exception
                        .insert_procedural_action_filter(
                            &hash,
                            &ProceduralOrActionFilter::from_css(s, st),
                        ),
                    LegacySpecificFilterType::ScriptInject(s) => {
                        inject_script.insert(&hash, (s, Default::default()))
                    }
                    LegacySpecificFilterType::UnhideScriptInject(s) => {
                        uninject_script.insert(&hash, s)
                    }
                }
            }
        }
        HostnameRuleDb {
            hide,
            unhide,
            inject_script,
            uninject_script,
            procedural_action,
            procedural_action_exception,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub(crate) struct LegacyRedirectResource {
    pub content_type: String,
    pub data: String,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Default)]
pub(crate) struct LegacyRedirectResourceStorage {
    #[serde(serialize_with = "stabilize_hashmap_serialization")]
    pub resources: HashMap<String, LegacyRedirectResource>,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct LegacyScriptletResource {
    scriptlet: String,
}

#[derive(Default, Deserialize, Serialize)]
pub(crate) struct LegacyScriptletResourceStorage {
    #[serde(serialize_with = "stabilize_hashmap_serialization")]
    resources: HashMap<String, LegacyScriptletResource>,
}

/// `_bug` is no longer used, and is removed from future format versions.
#[derive(Debug, Clone, Serialize)]
struct NetworkFilterSerializeFmt<'a> {
    mask: &'a crate::filters::network::NetworkFilterMask,
    filter: &'a crate::filters::network::FilterPart,
    opt_domains: &'a Option<Vec<crate::utils::Hash>>,
    opt_not_domains: &'a Option<Vec<crate::utils::Hash>>,
    redirect: &'a Option<String>,
    hostname: &'a Option<String>,
    csp: &'a Option<String>,
    _bug: Option<u32>,
    tag: &'a Option<String>,
    raw_line: Option<String>,
    id: &'a crate::utils::Hash,
    opt_domains_union: &'a Option<crate::utils::Hash>,
    opt_not_domains_union: &'a Option<crate::utils::Hash>,
}

/// Generic over `Borrow<NetworkFilter>` because `tagged_filters_all` requires `&'a NetworkFilter`
/// while `NetworkFilterList` requires `&'a Arc<NetworkFilter>`.
impl<'a, T> From<&'a T> for NetworkFilterSerializeFmt<'a>
where
    T: std::borrow::Borrow<NetworkFilter>,
{
    fn from(v: &'a T) -> NetworkFilterSerializeFmt<'a> {
        let v = v.borrow();
        NetworkFilterSerializeFmt {
            mask: &v.mask,
            filter: &v.filter,
            opt_domains: &v.opt_domains,
            opt_not_domains: &v.opt_not_domains,
            redirect: if v.is_redirect() {
                &v.modifier_option
            } else {
                &None
            },
            hostname: &v.hostname,
            csp: if v.is_csp() {
                &v.modifier_option
            } else {
                &None
            },
            _bug: None,
            tag: &v.tag,
            raw_line: v.raw_line.as_ref().map(|raw| *raw.clone()),
            id: &v.id,
            opt_domains_union: &v.opt_domains_union,
            opt_not_domains_union: &v.opt_not_domains_union,
        }
    }
}

/// Forces a `NetworkFilterList` to be serialized by converting to an
/// intermediate representation that is constructed with `NetworkFilterFmt` instead.
fn serialize_network_filter_list<S>(list: &NetworkFilterList, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    #[derive(Serialize, Default)]
    struct NetworkFilterListSerializeFmt {
        flatbuffer_memory: Vec<u8>,

        #[serde(serialize_with = "stabilize_hashmap_serialization")]
        filter_map: HashMap<Hash, Vec<u32>>,
    }

    let storage_list = NetworkFilterListSerializeFmt {
        flatbuffer_memory: list.flatbuffer_memory.clone(),
        filter_map: list.filter_map.clone(),
    };

    storage_list.serialize(s)
}

/// Forces a `NetworkFilter` slice to be serialized by converting to
/// an intermediate representation that is constructed with `NetworkFilterFmt` instead.
fn serialize_storage_network_filter_vec<S>(vec: &[NetworkFilter], s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let storage_vec: Vec<_> = vec.iter().map(NetworkFilterSerializeFmt::from).collect();

    storage_vec.serialize(s)
}

/// Provides structural aggregration of referenced adblock engine data to allow for allocation-free
/// serialization.
#[derive(Serialize)]
pub(crate) struct SerializeFormat<'a> {
    #[serde(serialize_with = "serialize_network_filter_list")]
    csp: &'a NetworkFilterList,
    #[serde(serialize_with = "serialize_network_filter_list")]
    exceptions: &'a NetworkFilterList,
    #[serde(serialize_with = "serialize_network_filter_list")]
    importants: &'a NetworkFilterList,
    #[serde(serialize_with = "serialize_network_filter_list")]
    redirects: &'a NetworkFilterList,
    #[serde(serialize_with = "serialize_network_filter_list")]
    filters_tagged: &'a NetworkFilterList,
    #[serde(serialize_with = "serialize_network_filter_list")]
    filters: &'a NetworkFilterList,
    #[serde(serialize_with = "serialize_network_filter_list")]
    generic_hide: &'a NetworkFilterList,

    #[serde(serialize_with = "serialize_storage_network_filter_vec")]
    tagged_filters_all: &'a Vec<NetworkFilter>,

    enable_optimizations: bool,

    resources: LegacyRedirectResourceStorage,

    #[serde(serialize_with = "stabilize_hashset_serialization")]
    simple_class_rules: &'a HashSet<String>,
    #[serde(serialize_with = "stabilize_hashset_serialization")]
    simple_id_rules: &'a HashSet<String>,
    #[serde(serialize_with = "stabilize_hashmap_serialization")]
    complex_class_rules: &'a HashMap<String, Vec<String>>,
    #[serde(serialize_with = "stabilize_hashmap_serialization")]
    complex_id_rules: &'a HashMap<String, Vec<String>>,

    specific_rules: LegacyHostnameRuleDb,

    #[serde(serialize_with = "stabilize_hashset_serialization")]
    misc_generic_selectors: &'a HashSet<String>,

    scriptlets: LegacyScriptletResourceStorage,

    #[serde(serialize_with = "stabilize_hashmap_serialization")]
    procedural_action: &'a HashMap<Hash, Vec<String>>,
    #[serde(serialize_with = "stabilize_hashmap_serialization")]
    procedural_action_exception: &'a HashMap<Hash, Vec<String>>,
}

impl<'a> SerializeFormat<'a> {
    pub fn serialize(&self) -> Result<Vec<u8>, SerializationError> {
        let mut output = super::ADBLOCK_RUST_DAT_MAGIC.to_vec();
        output.push(super::ADBLOCK_RUST_DAT_VERSION);
        rmps::encode::write(&mut output, &self)?;
        Ok(output)
    }
}

/// `_bug` is no longer used, and is cleaned up from future format versions.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct NetworkFilterDeserializeFmt {
    pub mask: crate::filters::network::NetworkFilterMask,
    pub filter: crate::filters::network::FilterPart,
    pub opt_domains: Option<Vec<crate::utils::Hash>>,
    pub opt_not_domains: Option<Vec<crate::utils::Hash>>,
    pub redirect: Option<String>,
    pub hostname: Option<String>,
    pub csp: Option<String>,
    _bug: Option<u32>,
    pub tag: Option<String>,
    pub raw_line: Option<String>,
    pub id: crate::utils::Hash,
    pub opt_domains_union: Option<crate::utils::Hash>,
    pub opt_not_domains_union: Option<crate::utils::Hash>,
}

impl From<NetworkFilterDeserializeFmt> for NetworkFilter {
    fn from(v: NetworkFilterDeserializeFmt) -> Self {
        Self {
            mask: v.mask,
            filter: v.filter,
            opt_domains: v.opt_domains,
            opt_not_domains: v.opt_not_domains,
            modifier_option: v.redirect.or(v.csp),
            hostname: v.hostname,
            tag: v.tag,
            raw_line: v.raw_line.map(Box::new),
            id: v.id,
            opt_domains_union: v.opt_domains_union,
            opt_not_domains_union: v.opt_not_domains_union,
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct NetworkFilterListDeserializeFmt {
    pub flatbuffer_memory: Vec<u8>,
    pub filter_map: HashMap<crate::utils::Hash, Vec<u32>>,
}

impl TryFrom<NetworkFilterListDeserializeFmt> for NetworkFilterList {
    fn try_from(v: NetworkFilterListDeserializeFmt) -> Result<Self, Self::Error> {
        let root = fb::root_as_network_filter_list(&v.flatbuffer_memory)?;
        // Reconstruct the unique_domains_hashes_map from the flatbuffer data
        let len = root.unique_domains_hashes().len();
        let mut unique_domains_hashes_map: HashMap<crate::utils::Hash, u16> =
            HashMap::with_capacity(len);
        for (index, hash) in root.unique_domains_hashes().iter().enumerate() {
            unique_domains_hashes_map.insert(
                hash,
                u16::try_from(index).map_err(|_| DeserializationError::FlatbufferSemanticError)?,
            );
        }
        Ok(Self {
            flatbuffer_memory: v.flatbuffer_memory,
            filter_map: v.filter_map,
            unique_domains_hashes_map,
        })
    }

    type Error = DeserializationError;
}

/// Structural representation of adblock engine data that can be built up from deserialization and
/// used directly to construct new `Engine` components without unnecessary allocation.
#[derive(Deserialize)]
pub(crate) struct DeserializeFormat {
    csp: NetworkFilterListDeserializeFmt,
    exceptions: NetworkFilterListDeserializeFmt,
    importants: NetworkFilterListDeserializeFmt,
    redirects: NetworkFilterListDeserializeFmt,
    filters_tagged: NetworkFilterListDeserializeFmt,
    filters: NetworkFilterListDeserializeFmt,
    generic_hide: NetworkFilterListDeserializeFmt,

    tagged_filters_all: Vec<NetworkFilterDeserializeFmt>,

    enable_optimizations: bool,

    _resources: LegacyRedirectResourceStorage,

    simple_class_rules: HashSet<String>,
    simple_id_rules: HashSet<String>,
    complex_class_rules: HashMap<String, Vec<String>>,
    complex_id_rules: HashMap<String, Vec<String>>,

    specific_rules: LegacyHostnameRuleDb,

    misc_generic_selectors: HashSet<String>,

    _scriptlets: LegacyScriptletResourceStorage,

    #[serde(default)]
    procedural_action: HashMap<Hash, Vec<String>>,
    #[serde(default)]
    procedural_action_exception: HashMap<Hash, Vec<String>>,
}

impl DeserializeFormat {
    pub fn deserialize(serialized: &[u8]) -> Result<Self, DeserializationError> {
        let data = super::parse_dat_header(serialized)?;
        let format: Self = rmps::decode::from_read(data)?;
        Ok(format)
    }
}

impl<'a> From<(&'a Blocker, &'a CosmeticFilterCache)> for SerializeFormat<'a> {
    fn from(v: (&'a Blocker, &'a CosmeticFilterCache)) -> Self {
        let (blocker, cfc) = v;
        Self {
            csp: &blocker.csp,
            exceptions: &blocker.exceptions,
            importants: &blocker.importants,
            redirects: &blocker.redirects,
            filters_tagged: &blocker.filters_tagged,
            filters: &blocker.filters,
            generic_hide: &blocker.generic_hide,

            tagged_filters_all: &blocker.tagged_filters_all,

            enable_optimizations: blocker.enable_optimizations,

            resources: LegacyRedirectResourceStorage::default(),

            simple_class_rules: &cfc.simple_class_rules,
            simple_id_rules: &cfc.simple_id_rules,
            complex_class_rules: &cfc.complex_class_rules,
            complex_id_rules: &cfc.complex_id_rules,

            specific_rules: (&cfc.specific_rules).into(),

            misc_generic_selectors: &cfc.misc_generic_selectors,

            scriptlets: LegacyScriptletResourceStorage::default(),

            procedural_action: &cfc.specific_rules.procedural_action.0,
            procedural_action_exception: &cfc.specific_rules.procedural_action_exception.0,
        }
    }
}

impl TryFrom<DeserializeFormat> for (Blocker, CosmeticFilterCache) {
    fn try_from(v: DeserializeFormat) -> Result<Self, Self::Error> {
        use crate::cosmetic_filter_cache::HostnameFilterBin;

        let mut specific_rules: HostnameRuleDb = v.specific_rules.into();
        specific_rules.procedural_action = HostnameFilterBin(v.procedural_action);
        specific_rules.procedural_action_exception =
            HostnameFilterBin(v.procedural_action_exception);

        Ok((
            Blocker {
                csp: v.csp.try_into()?,
                exceptions: v.exceptions.try_into()?,
                importants: v.importants.try_into()?,
                redirects: v.redirects.try_into()?,
                removeparam: NetworkFilterList::default(),
                filters_tagged: v.filters_tagged.try_into()?,
                filters: v.filters.try_into()?,
                generic_hide: v.generic_hide.try_into()?,

                tags_enabled: Default::default(),
                tagged_filters_all: v.tagged_filters_all.into_iter().map(|f| f.into()).collect(),

                enable_optimizations: v.enable_optimizations,
                regex_manager: Default::default(),
            },
            CosmeticFilterCache {
                simple_class_rules: v.simple_class_rules,
                simple_id_rules: v.simple_id_rules,
                complex_class_rules: v.complex_class_rules,
                complex_id_rules: v.complex_id_rules,

                specific_rules,

                misc_generic_selectors: v.misc_generic_selectors,
            },
        ))
    }

    type Error = DeserializationError;
}
