use aws_config::meta::region::RegionProviderChain;
use aws_config::Region;
use aws_sdk_textract::types::{Block, BlockType, Document, EntityType, RelationshipType};
use aws_sdk_textract::Client;
use std::collections::HashMap;

#[derive(Debug)]
struct KeyValuePair {
    key: String,
    value: String,
    key_bounding_box: Option<BoundingBox>,
    value_bounding_box: Option<BoundingBox>,
}

#[derive(Debug, Clone)]
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
    let key_value_pairs = extract_key_value_pairs(blocks);

    // Print the extracted key-value pairs
    for pair in key_value_pairs {
        println!("Key: {}", pair.key);
        println!("Value: {}", pair.value);
        println!("Key Bounding Box: {:?}", pair.key_bounding_box);
        println!("Value Bounding Box: {:?}", pair.value_bounding_box);
        println!("---");
    }

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
