//! Tags the bills' lines get on top of their own columns, so Cloud Cost's pages can find them:
//!
//! - AWS: Datadog's `aws_product` (the product code without its `Amazon` / `AWS` prefix, lower-cased:
//!   `AmazonS3` → `s3`), `aws_product_family` (the report's own, else read from the usage type) and
//!   `aws_datatransfer_type` / `_direction`, also as the providers' common `data_transfer_type` (Azure's
//!   bandwidth and global peering too), and S3's `storage_class`.
//! - AI: `ai_vendor` and `ai_model` of model inference (Bedrock, its Marketplace models, Azure OpenAI and
//!   the other Foundry models), read from the usage type or the meter name.

use std::collections::BTreeMap;

/// Tags of an AWS line item from its product code, usage type and product family (empty when the
/// report has no such column).
pub fn aws(tags: &mut BTreeMap<String, String>, product: &str, usage_type: &str, family: &str) {
    let mut set = |key: &str, value: &str| {
        if !value.is_empty() {
            tags.insert(key.to_string(), value.to_string());
        }
    };
    let code = product.to_ascii_lowercase();
    set("aws_product", ["amazon", "aws"].iter().find_map(|p| code.strip_prefix(p).filter(|rest| !rest.is_empty())).unwrap_or(&code));
    let usage = without_region(usage_type);
    let transfer = transfer(usage);
    if let Some((kind, direction)) = transfer {
        set("aws_datatransfer_type", kind);
        set("data_transfer_type", kind);
        set("aws_datatransfer_direction", direction);
    }
    let derived = match usage {
        u if u.contains("NatGateway") => "NAT Gateway",
        u if u.contains("LoadBalancerUsage") || u.contains("LCUUsage") => "Load Balancer",
        u if u.contains("VpcEndpoint") => "VpcEndpoint",
        u if u.contains("PublicIPv4") => "IP Address",
        _ if transfer.is_some() => "Data Transfer",
        _ => "",
    };
    set("aws_product_family", if family.is_empty() { derived } else { family });
    if product == "AmazonS3" {
        set("storage_class", usage.strip_prefix("TimedStorage-").map_or("", storage_class));
    }
    // Bedrock's own models (`USE1-Llama3-3-70B-input-tokens`), and the Marketplace ones billed under a
    // product named after the model.
    let model = match product.strip_suffix(" (Amazon Bedrock Edition)") {
        Some(model) => Some(model),
        None if product == "AmazonBedrock" => ["-input", "-output", "-cache", "-batch"].iter().filter_map(|m| usage.to_ascii_lowercase().find(m)).min().map(|end| &usage[..end]),
        None => None,
    };
    if product == "AmazonBedrock" || model.is_some() {
        set("ai_vendor", "Amazon Bedrock");
        set("ai_model", model.unwrap_or_default());
    }
}

/// A usage type without its region prefix (`USE1-`, `EUW2-`); global usage types have none.
fn without_region(usage_type: &str) -> &str {
    match usage_type.split_once('-') {
        Some((region, rest)) if (2..=5).contains(&region.len()) && region.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()) => rest,
        _ => usage_type,
    }
}

/// S3's storage class of a `TimedStorage-<class>ByteHrs` usage type.
fn storage_class(rest: &str) -> &'static str {
    match rest.trim_end_matches("ByteHrs").trim_end_matches('-') {
        "" => "standard",
        "Glacier" => "glacier",
        "GIR" => "glacier-ir",
        "GDA" => "deep-archive",
        "SIA" => "standard-ia",
        "ZIA" => "onezone-ia",
        "RRS" => "reduced-redundancy",
        c if c.starts_with("INT") => "intelligent-tiering",
        _ => "",
    }
}

/// The type and direction of data transfer usage (`DataTransfer-Out-Bytes`, `EU-AWS-Out-Bytes`).
fn transfer(usage: &str) -> Option<(&'static str, &'static str)> {
    let direction = if usage.contains("-Out-") { "out" } else if usage.contains("-In-") { "in" } else { "" };
    match usage {
        u if u.contains("DataTransfer-Regional-Bytes") => Some(("cross-zone", "")),
        u if u.ends_with("-AWS-Out-Bytes") || u.ends_with("-AWS-In-Bytes") => Some(("cross-region", direction)),
        u if u.contains("DataTransfer-Out-Bytes") || u.contains("DataTransfer-In-Bytes") => Some(("internet", direction)),
        _ => None,
    }
}

/// Tags of an Azure row from its meter category, subcategory and meter name.
pub fn azure(tags: &mut BTreeMap<String, String>, category: &str, subcategory: &str, meter: &str) {
    let transfer = match category {
        "Bandwidth" if subcategory.contains("Inter-Region") => "cross-region",
        "Bandwidth" if meter.contains("Data Transfer") => "internet",
        "Virtual Network" if subcategory.starts_with("Global Virtual Network Peering") => "cross-region",
        _ => "",
    };
    if !transfer.is_empty() {
        tags.insert("data_transfer_type".into(), transfer.into());
    }
    let openai = subcategory.to_ascii_lowercase().contains("openai");
    let vendor = match category {
        "Foundry Models" | "Cognitive Services" if openai => "Azure OpenAI",
        "Foundry Models" => "Azure AI Foundry",
        _ => return,
    };
    tags.insert("ai_vendor".into(), vendor.into());
    // The meter's name up to its kind of tokens: `gpt 4.1 cached Inp glbl Tokens`, `5.4 mini Opt Gl 1M Tokens`.
    const KINDS: [&str; 18] = ["inp", "input", "outp", "output", "opt", "cached", "cd", "cchd", "shortco", "longco", "batch", "glbl", "gl", "rgnl", "dzone", "std", "tokens", "1m"];
    let words: Vec<String> = meter.split([' ', '-']).filter(|w| !w.is_empty()).map(str::to_ascii_lowercase).take_while(|w| !KINDS.contains(&w.as_str())).collect();
    let model = words.join("-");
    if !model.is_empty() {
        // The GPT-5 and later meters leave out the family's name (`5.4 nano`).
        let gpt = openai && model.starts_with(|c: char| c.is_ascii_digit());
        tags.insert("ai_model".into(), if gpt { format!("gpt-{model}") } else { model });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aws_tags(product: &str, usage_type: &str, family: &str) -> BTreeMap<String, String> {
        let mut tags = BTreeMap::new();
        aws(&mut tags, product, usage_type, family);
        tags
    }

    fn azure_tags(category: &str, subcategory: &str, meter: &str) -> BTreeMap<String, String> {
        let mut tags = BTreeMap::new();
        azure(&mut tags, category, subcategory, meter);
        tags
    }

    fn get<'a>(tags: &'a BTreeMap<String, String>, key: &str) -> Option<&'a str> {
        tags.get(key).map(String::as_str)
    }

    #[test]
    fn aws_products_are_named_as_datadog_names_them() {
        for (code, product) in [("AmazonS3", "s3"), ("AmazonEC2", "ec2"), ("AWSELB", "elb"), ("AWSDataTransfer", "datatransfer"), ("awswaf", "waf"), ("ACM", "acm"), ("AmazonBedrock", "bedrock")] {
            assert_eq!(get(&aws_tags(code, "", ""), "aws_product"), Some(product), "{code}");
        }
    }

    #[test]
    fn networking_usage_gets_a_product_family() {
        for (product, usage_type, family) in [
            ("AmazonEC2", "NatGateway-Bytes", "NAT Gateway"),
            ("AmazonEC2", "USW2-NatGateway-Hours", "NAT Gateway"),
            ("AWSELB", "DataTransfer-Out-Bytes", "Data Transfer"),
            ("AmazonEC2", "USE1-EU-AWS-Out-Bytes", "Data Transfer"),
            ("AWSELB", "LoadBalancerUsage", "Load Balancer"),
            ("AWSELB", "USW2-LCUUsage", "Load Balancer"),
            ("AmazonVPC", "USE1-VpcEndpoint-Hours", "VpcEndpoint"),
            ("AmazonVPC", "USE1-PublicIPv4:InUseAddress", "IP Address"),
        ] {
            assert_eq!(get(&aws_tags(product, usage_type, ""), "aws_product_family"), Some(family), "{usage_type}");
        }
        // The report's own family wins; usage it doesn't classify gets none.
        assert_eq!(get(&aws_tags("AWSELB", "LoadBalancerUsage", "Load Balancer-Application"), "aws_product_family"), Some("Load Balancer-Application"));
        assert_eq!(get(&aws_tags("AmazonS3", "TimedStorage-ByteHrs", ""), "aws_product_family"), None);
    }

    #[test]
    fn data_transfer_gets_its_type_and_direction() {
        for (usage_type, kind, direction) in [
            ("DataTransfer-Regional-Bytes", "cross-zone", None),
            ("USE1-EU-AWS-Out-Bytes", "cross-region", Some("out")),
            ("USE1-CAN1-AWS-In-Bytes", "cross-region", Some("in")),
            ("EUW2-DataTransfer-Out-Bytes", "internet", Some("out")),
            ("DataTransfer-In-Bytes", "internet", Some("in")),
        ] {
            let tags = aws_tags("AmazonEC2", usage_type, "");
            assert_eq!((get(&tags, "aws_datatransfer_type"), get(&tags, "data_transfer_type")), (Some(kind), Some(kind)), "{usage_type}");
            assert_eq!(get(&tags, "aws_datatransfer_direction"), direction, "{usage_type}");
        }
        assert_eq!(get(&aws_tags("AmazonEC2", "NatGateway-Bytes", ""), "data_transfer_type"), None);
    }

    #[test]
    fn s3_storage_gets_its_class() {
        for (usage_type, class) in [("TimedStorage-ByteHrs", "standard"), ("USE2-TimedStorage-GlacierByteHrs", "glacier"), ("TimedStorage-SIA-ByteHrs", "standard-ia"), ("TimedStorage-INT-FA-ByteHrs", "intelligent-tiering"), ("TimedStorage-GDA-ByteHrs", "deep-archive")] {
            assert_eq!(get(&aws_tags("AmazonS3", usage_type, ""), "storage_class"), Some(class), "{usage_type}");
        }
        assert_eq!(get(&aws_tags("AmazonS3", "Requests-Tier1", ""), "storage_class"), None);
    }

    #[test]
    fn azure_data_transfer_gets_its_type() {
        for (category, subcategory, meter, kind) in [
            ("Bandwidth", "Rtn Preference: MGN", "Standard Data Transfer Out", "internet"),
            ("Bandwidth", "Bandwidth Inter-Region", "Intra Continent Data Transfer Out", "cross-region"),
            ("Virtual Network", "Global Virtual Network Peering", "Inter-Region Egress", "cross-region"),
        ] {
            assert_eq!(get(&azure_tags(category, subcategory, meter), "data_transfer_type"), Some(kind), "{meter}");
        }
        assert_eq!(get(&azure_tags("NAT Gateway", "NAT Gateway", "Standard Data Processed"), "data_transfer_type"), None);
    }

    #[test]
    fn bedrock_inference_names_its_model() {
        let tags = aws_tags("AmazonBedrock", "USE1-Llama3-3-70B-input-tokens", "");
        assert_eq!((get(&tags, "ai_vendor"), get(&tags, "ai_model")), (Some("Amazon Bedrock"), Some("Llama3-3-70B")));
        assert_eq!(get(&aws_tags("AmazonBedrock", "USE1-Llama3-3-70B-output-tokens", ""), "ai_model"), Some("Llama3-3-70B"));
        // Marketplace models (Anthropic's, …) bill under their own product, named after the model.
        let tags = aws_tags("Claude 3.7 Sonnet (Amazon Bedrock Edition)", "USE1-MP:USE1_InputTokenCount-Units", "");
        assert_eq!((get(&tags, "ai_vendor"), get(&tags, "ai_model")), (Some("Amazon Bedrock"), Some("Claude 3.7 Sonnet")));
        assert_eq!(get(&aws_tags("AmazonEC2", "BoxUsage:m6i.xlarge", ""), "ai_vendor"), None);
    }

    #[test]
    fn azure_openai_meters_name_their_model() {
        for (subcategory, meter, model) in [
            ("Azure OpenAI", "gpt 4.1 cached Inp glbl Tokens", "gpt-4.1"),
            ("Azure OpenAI", "gpt 4.1 Outp glbl Tokens", "gpt-4.1"),
            ("Azure OpenAI", "gpt-4o-0806-Inp-glbl Tokens", "gpt-4o-0806"),
            ("Azure OpenAI GPT5", "GPT 5.1 opt Gl 1M Tokens", "gpt-5.1"),
            ("Azure OpenAI GPT5", "5.4 mini cd Inp Gl 1M Tokens", "gpt-5.4-mini"),
            ("Azure OpenAI GPT6", "6-luna ShortCo Inp Std Gl 1M Tokens", "gpt-6-luna"),
        ] {
            let tags = azure_tags("Foundry Models", subcategory, meter);
            assert_eq!((get(&tags, "ai_vendor"), get(&tags, "ai_model")), (Some("Azure OpenAI"), Some(model)), "{meter}");
        }
        // Older bills put Azure OpenAI under Cognitive Services; other Foundry models are Azure AI Foundry's.
        assert_eq!(get(&azure_tags("Cognitive Services", "Azure OpenAI", "gpt-4o-0513-Inp-glbl Tokens"), "ai_vendor"), Some("Azure OpenAI"));
        assert_eq!(get(&azure_tags("Foundry Models", "Llama", "Llama 3.3 70B Inp Tokens"), "ai_vendor"), Some("Azure AI Foundry"));
        assert_eq!(get(&azure_tags("Cognitive Services", "Speech", "S1 Audio Hours"), "ai_vendor"), None);
        assert!(azure_tags("Virtual Machines", "Dv5 Series", "D4s v5").is_empty());
    }
}
