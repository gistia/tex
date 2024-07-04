use aws_config::meta::region::RegionProviderChain;
use aws_config::Region;
use aws_sdk_textract::types::{Block, BlockType, Document, EntityType, RelationshipType};
use aws_sdk_textract::Client;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Serialize)]
struct KeyValuePair {
    key: String,
    value: String,
    key_bounding_box: Option<BoundingBox>,
    value_bounding_box: Option<BoundingBox>,
}

#[derive(Debug, Clone, Serialize)]
struct BoundingBox {
    width: f32,
    height: f32,
    left: f32,
    top: f32,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up the AWS region
    let region_provider = RegionProviderChain::default_provider().or_else(Region::new("us-east-1"));

    // Load configuration
    #[allow(deprecated)]
    let config = aws_config::from_env().region(region_provider).load().await;

    // Create a Textract client
    let client = Client::new(&config);

    // Specify the S3 bucket and document
    let bucket = "smartflow-dev";
    let document_name = "surgical-form-felipe.jpg";

    // Create the Document object
    let document = Document::builder()
        .s3_object(
            aws_sdk_textract::types::S3Object::builder()
                .bucket(bucket)
                .name(document_name)
                .build(),
        )
        .build();

    // Call Textract to analyze the document
    let resp = client
        .analyze_document()
        .feature_types("FORMS".into())
        .document(document)
        .send()
        .await?;

    // Process the results
    let blocks = resp.blocks();
    let mut key_value_pairs = extract_key_value_pairs(blocks);

    // Sort the key_value_pairs
    sort_key_value_pairs(&mut key_value_pairs);

    serde_json::to_writer_pretty(std::io::stdout(), &key_value_pairs)?;

    Ok(())
}

fn extract_key_value_pairs(blocks: &[Block]) -> Vec<KeyValuePair> {
    let mut key_map = HashMap::new();
    let mut value_map = HashMap::new();
    let mut block_map = HashMap::new();

    for block in blocks {
        if let Some(block_id) = block.id() {
            block_map.insert(block_id.to_string(), block);

            if block.block_type() == Some(&BlockType::KeyValueSet) {
                if block.entity_types().contains(&EntityType::Key) {
                    key_map.insert(block_id.to_string(), block);
                } else {
                    value_map.insert(block_id.to_string(), block);
                }
            }
        }
    }

    let mut key_value_pairs = Vec::new();

    for (_, key_block) in key_map {
        let key_text = get_text_for_block(key_block, &block_map);
        let key_bounding_box = key_block
            .geometry()
            .and_then(|g| g.bounding_box())
            .map(|bb| BoundingBox {
                width: bb.width(),
                height: bb.height(),
                left: bb.left(),
                top: bb.top(),
            });

        let relationships = key_block.relationships();
        for relationship in relationships {
            if relationship.r#type() == Some(&RelationshipType::Value) {
                for value_block_id in relationship.ids() {
                    if let Some(value_block) = value_map.get(value_block_id) {
                        let value_text = get_text_for_block(value_block, &block_map);
                        let value_bounding_box = value_block
                            .geometry()
                            .and_then(|g| g.bounding_box())
                            .map(|bb| BoundingBox {
                                width: bb.width(),
                                height: bb.height(),
                                left: bb.left(),
                                top: bb.top(),
                            });

                        key_value_pairs.push(KeyValuePair {
                            key: key_text.clone(),
                            value: value_text,
                            key_bounding_box: key_bounding_box.clone(),
                            value_bounding_box,
                        });
                    }
                }
            }
        }
    }

    key_value_pairs
}

fn get_text_for_block(block: &Block, block_map: &HashMap<String, &Block>) -> String {
    let mut text = String::new();

    let relationships = block.relationships();
    for relationship in relationships {
        if relationship.r#type() == Some(&RelationshipType::Child) {
            for child_id in relationship.ids() {
                if let Some(word_block) = block_map.get(child_id) {
                    if word_block.block_type() == Some(&BlockType::Word) {
                        if let Some(word_text) = word_block.text() {
                            text.push_str(word_text);
                            text.push(' ');
                        }
                    }
                }
            }
        }
    }

    text.trim().to_string()
}

fn sort_key_value_pairs(key_value_pairs: &mut Vec<KeyValuePair>) {
    key_value_pairs.sort_by(|a, b| {
        let a_box = a
            .key_bounding_box
            .as_ref()
            .or(a.value_bounding_box.as_ref());
        let b_box = b
            .key_bounding_box
            .as_ref()
            .or(b.value_bounding_box.as_ref());

        match (a_box, b_box) {
            (Some(a), Some(b)) => a
                .top
                .partial_cmp(&b.top)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    a.left
                        .partial_cmp(&b.left)
                        .unwrap_or(std::cmp::Ordering::Equal)
                }),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    });
}
