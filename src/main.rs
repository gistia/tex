use aws_config::meta::region::RegionProviderChain;
use aws_config::Region;
use aws_sdk_textract::types::{Block, BlockType, Document, EntityType, RelationshipType, S3Object};
use aws_sdk_textract::Client;

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
            S3Object::builder()
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
    for block in blocks {
        match block.block_type().unwrap() {
            BlockType::KeyValueSet => {
                // Process key-value pairs (form fields)
                if block.entity_types().contains(&EntityType::Key) {
                    let key = block.text().unwrap_or("Unknown");
                    let value = find_value_block(blocks, block).unwrap_or("N/A");
                    println!("Field: {}, Value: {}", key, value);
                }
            }
            BlockType::Word => {
                // Process individual words and their bounding boxes
                if let Some(geometry) = block.geometry() {
                    if let Some(bounding_box) = geometry.bounding_box() {
                        println!(
                            "Word: {}, Bounding Box: {:?}",
                            block.text().unwrap_or("Unknown"),
                            bounding_box
                        );
                    }
                }
            }
            _ => {}
        }
    }

    Ok(())
}

// Helper function to find the value block for a given key block
fn find_value_block<'a>(blocks: &'a [Block], key_block: &Block) -> Option<&'a str> {
    let value_ids = key_block
        .relationships()
        .iter()
        .find(|rel| rel.r#type() == Some(&RelationshipType::Value))
        .and_then(|rel| Some(rel.ids()));

    if let Some(ids) = value_ids {
        for id in ids {
            if let Some(block) = blocks.iter().find(|b| b.id() == Some(id)) {
                return block.text();
            }
        }
    }

    None
}
