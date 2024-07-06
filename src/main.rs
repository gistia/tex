use aws_config::meta::region::RegionProviderChain;
use aws_config::Region;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_textract::types::{Block, BlockType, Document, EntityType, RelationshipType};
use aws_sdk_textract::Client as TextractClient;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use image::{ImageBuffer, Rgb};
use imageproc::drawing::{draw_hollow_rect_mut, draw_text_mut};
use imageproc::rect::Rect;
use rusttype::{Font, Scale};
use serde::Serialize;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

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

struct AppState {
    textract_client: TextractClient,
    s3_client: S3Client,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Set up the AWS region
    let region_provider = RegionProviderChain::default_provider().or_else(Region::new("us-east-1"));

    // Load configuration
    #[allow(deprecated)]
    let config = aws_config::from_env().region(region_provider).load().await;

    // Create a Textract client
    let textract_client = TextractClient::new(&config);
    let s3_client = S3Client::new(&config);

    // Create app state
    let app_state = Arc::new(AppState {
        textract_client,
        s3_client,
    });

    // Build our application with a route
    let app = Router::new()
        .route("/analyze/:image_name", get(analyze_image))
        .route("/display/:image_name", get(display_image))
        .with_state(app_state);

    // Run our application
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 3001));
    println!("Listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn display_image(
    Path(image_name): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let bucket = "smartflow-dev";

    // Fetch image from S3
    let get_object_output = state
        .s3_client
        .get_object()
        .bucket(bucket)
        .key(&image_name)
        .send()
        .await
        .unwrap();

    let image_data = get_object_output.body.collect().await.unwrap().into_bytes();
    let mut img = image::load_from_memory(&image_data).unwrap().to_rgb8();

    // Analyze the document with Textract
    let document = Document::builder()
        .s3_object(
            aws_sdk_textract::types::S3Object::builder()
                .bucket(bucket)
                .name(&image_name)
                .build(),
        )
        .build();

    let resp = state
        .textract_client
        .analyze_document()
        .feature_types("FORMS".into())
        .document(document)
        .send()
        .await
        .unwrap();

    let blocks = resp.blocks();
    let key_value_pairs = extract_key_value_pairs(blocks);

    // Draw bounding boxes
    let font = Vec::from(include_bytes!("roboto.ttf") as &[u8]);
    let font = Font::try_from_vec(font).unwrap();

    for pair in key_value_pairs {
        if let Some(key_box) = pair.key_bounding_box {
            draw_bounding_box(&mut img, &key_box, Rgb([0, 0, 255]), 3); // Blue for keys
            draw_text(&mut img, &pair.key, &key_box, Rgb([0, 0, 0]), &font);
        }
        if let Some(value_box) = pair.value_bounding_box {
            draw_bounding_box(&mut img, &value_box, Rgb([255, 0, 0]), 3); // Red for values
            draw_text(&mut img, &pair.value, &value_box, Rgb([255, 0, 0]), &font);
        }
    }

    // Convert image to bytes
    let mut buffer = Cursor::new(Vec::new());
    img.write_to(&mut buffer, image::ImageOutputFormat::Png)
        .unwrap();

    // Return the image
    (
        [(axum::http::header::CONTENT_TYPE, "image/png")],
        buffer.into_inner(),
    )
}

fn draw_bounding_box(
    img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>,
    bbox: &BoundingBox,
    color: Rgb<u8>,
    thickness: u8,
) {
    let base_rect = Rect::at(
        (bbox.left * img.width() as f32) as i32,
        (bbox.top * img.height() as f32) as i32,
    )
    .of_size(
        (bbox.width * img.width() as f32) as u32,
        (bbox.height * img.height() as f32) as u32,
    );

    // Draw multiple rectangles to create a thicker border
    for i in 0..thickness {
        let offset = i as i32;
        let expanded_rect = Rect::at(base_rect.left() - offset, base_rect.top() - offset).of_size(
            base_rect.width() + 2 * offset as u32,
            base_rect.height() + 2 * offset as u32,
        );
        draw_hollow_rect_mut(img, expanded_rect, color);
    }
}

fn draw_text(
    img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>,
    text: &str,
    bbox: &BoundingBox,
    color: Rgb<u8>,
    font: &Font,
) {
    let scale = Scale::uniform(12.0);
    let x = (bbox.left * img.width() as f32) as i32;
    let y = (bbox.top * img.height() as f32) as i32;
    draw_text_mut(img, color, x, y, scale, font, text);
}

async fn analyze_image(
    Path(image_name): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Json<Vec<KeyValuePair>> {
    // Specify the S3 bucket and document
    let bucket = "smartflow-dev";

    // Create the Document object
    let document = Document::builder()
        .s3_object(
            aws_sdk_textract::types::S3Object::builder()
                .bucket(bucket)
                .name(&image_name)
                .build(),
        )
        .build();

    // Call Textract to analyze the document
    let resp = state
        .textract_client
        .analyze_document()
        .feature_types("FORMS".into())
        .document(document)
        .send()
        .await
        .unwrap();

    // Process the results
    let blocks = resp.blocks();
    let mut key_value_pairs = extract_key_value_pairs(blocks);

    // Sort the key_value_pairs
    sort_key_value_pairs(&mut key_value_pairs);

    Json(key_value_pairs)
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
