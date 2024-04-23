// tests.rs

#[cfg(test)]
mod tests_tree_sitter {
    use std::time;
    use tree_sitter::{Parser, Point, Query, QueryCursor, QueryMatches, Range, TreeCursor};

    fn walk_tree(cursor: &mut TreeCursor, source_code: &str) {
        let node = cursor.node();
        println!("Node kind: {:?}", node.kind());

        let start_byte = node.start_byte();
        let end_byte = node.end_byte();
        let node_text = &source_code[start_byte..end_byte];
        println!("Node text: {:?}", node_text);

        if cursor.goto_first_child() {
            walk_tree(cursor, source_code);
            cursor.goto_parent();
        }

        while cursor.goto_next_sibling() {
            walk_tree(cursor, source_code);
        }
    }

    #[test]
    fn test_tree_sitter_walk() {
        let mut parser = Parser::new();

        let language = tree_sitter_rust::language();
        parser
            .set_language(language)
            .expect("Error loading Rust grammar");

        let source_code = r"
        fn test() {
            let a = 5;
        }
        ";

        let start = time::Instant::now();
        let tree = parser.parse(source_code, None).unwrap();
        let elapsed = time::Instant::now() - start;

        println!("Elapsed time: {:?} ms", elapsed.as_millis());

        let mut cursor = tree.walk();
        walk_tree(&mut cursor, source_code);
    }

    #[test]
    fn test_tree_sitter_query() {
        let mut parser = Parser::new();

        let language = tree_sitter_rust::language();
        parser
            .set_language(language)
            .expect("Error loading Rust grammar");

        let source_code = r#"
            fn foo() {
                let x = 42;
                println!("Hello, world!");
            }
        "#;

        let start = time::Instant::now();
        let tree = parser.parse(source_code, None).unwrap();
        let elapsed = time::Instant::now() - start;

        println!("Elapsed time: {:?} ms", elapsed.as_millis());

        let query_pattern = r#"
        [
          (string_literal)
          (raw_string_literal)
        ] @string

        (function_item
            name: (identifier) @function)

        "fn" @keyword.function
        "#;

        let query = Query::new(language, query_pattern).unwrap();
        let mut query_cursor = QueryCursor::new();
        // query_cursor.set_byte_range(0..source_code.len());
        query_cursor.set_byte_range(0..38);

        let text_provider = |node: tree_sitter::Node| {
            println!("text_provider");
            let range = node.byte_range();
            let text_slice = &source_code.as_bytes()[range.start..range.end];
            let iter = vec![text_slice].into_iter();
            iter
        };

        let dummy = |node: tree_sitter::Node| vec![].into_iter();
        let source_code_bytes = &source_code.as_bytes();
        let start = time::Instant::now();

        let matches = query_cursor.matches(&query, tree.root_node(), dummy);

        for qmatch in matches {
            for capture in qmatch.captures {
                match capture.node.utf8_text(source_code_bytes) {
                    Ok(text) => {
                        let i = capture.index as usize;
                        let capture_name = &query.capture_names()[i];
                        let text = format!("\x1b[{}m{}\x1b[0m", i + 100, text);
                        println!("{:20} {}", capture_name, text);
                    }
                    _ => {}
                };
            }
        }

        let elapsed = time::Instant::now() - start;
        println!("Elapsed time: {:?} ms", elapsed.as_millis());
    }

    #[test]
    fn test_tree_sitter_colors_ranges() {
        let mut parser = Parser::new();

        let language = tree_sitter_rust::language();
        parser
            .set_language(language)
            .expect("Error loading Rust grammar");

        let source_code = r#"
fn foo() {
    let x = 42;
    println!("Hello, world!");
}
        "#;

        let tree = parser.parse(source_code, None).unwrap();

        let query_pattern = r#"
        [
          (string_literal)
          (raw_string_literal)
        ] @string

        (function_item
            name: (identifier) @function)

        "fn" @keyword.function
        "#;

        let query = Query::new(language, query_pattern).unwrap();
        let mut query_cursor = QueryCursor::new();
        query_cursor.set_byte_range(0..source_code.len());
        // query_cursor.set_byte_range(0..38);
        // query_cursor.set_byte_range(0..3);

        let dummy = |node: tree_sitter::Node| vec![].into_iter();
        let source_code_bytes = &source_code.as_bytes();
        let start = time::Instant::now();

        let matches = query_cursor.matches(&query, tree.root_node(), dummy);

        let mut color_ranges: Vec<(Point, Point, usize)> = vec![];

        for qmatch in matches {
            for capture in qmatch.captures {
                let i = capture.index as usize;
                let capture_name = &query.capture_names()[i];

                let color_range = (
                    capture.node.start_position(),
                    capture.node.end_position(),
                    i,
                );
                color_ranges.push(color_range);
            }
        }

        let elapsed = time::Instant::now() - start;
        println!("Elapsed time: {:?} ns", elapsed.as_nanos());

        color_ranges.iter().for_each(|cr| println!("{:?}", cr));
    }
}

#[cfg(test)]
mod tests_text {
    use crate::code::Code;

    #[test]
    fn test_new_text_buffer_is_empty() {
        let text_buffer = Code::new();
        assert!(text_buffer.is_empty());
    }

    #[test]
    fn test_new_text_buffer_is_zero_len() {
        let text_buffer = Code::new();
        assert_eq!(text_buffer.len_lines(), 1);
    }

    #[test]
    fn test_insert_char() {
        let mut text_buffer = Code::new();
        text_buffer.insert_char('a', 0, 0);
        assert_eq!(text_buffer.len_lines(), 1);
        assert_eq!(text_buffer.line_len(0), 1);
    }

    #[test]
    fn test_remove_char() {
        let mut text_buffer = Code::new();
        text_buffer.insert_char('a', 0, 0);
        text_buffer.remove_char(0, 1);
        assert_eq!(text_buffer.len_lines(), 1);
    }
}

#[cfg(test)]
mod tests_selection {
    use crate::selection::{Point, Selection};

    #[test]
    fn test_is_selected_when_active_and_inside_selection() {
        let mut selection = Selection::new();
        selection.start = Point { y: 1, x: 1 };
        selection.end = Point { y: 3, x: 3 };
        selection.active = true;

        assert!(selection.contains(2, 2));
        assert!(selection.contains(1, 1));
        assert!(selection.contains(3, 2));
    }

    #[test]
    fn test_is_selected_when_active_and_outside_selection() {
        let mut selection = Selection::new();
        selection.start = Point { y: 1, x: 1 };
        selection.end = Point { y: 3, x: 3 };
        selection.active = true;

        assert!(!selection.is_selected(0, 0));
        assert!(!selection.is_selected(4, 4));
        assert!(selection.is_selected(1, 4));
        assert!(!selection.is_selected(4, 1));
    }

    #[test]
    fn test_is_selected_when_inactive() {
        let mut selection = Selection::new();

        assert!(!selection.contains(2, 2));
        assert!(!selection.contains(1, 1));
        assert!(!selection.contains(3, 3));
    }
}

#[cfg(test)]
mod tokio_test {

    use std::process::Stdio;
    use tokio::io;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Command;

    #[tokio::test]
    async fn test_process() -> io::Result<()> {
        let mut cmd = Command::new("ls");

        cmd.stdout(Stdio::piped());

        let mut child = cmd.spawn().expect("failed to spawn command");

        let stdout = child.stdout.take().expect("can not get stdout");

        let mut reader = BufReader::new(stdout).lines();

        let status = child.wait().await?;

        println!("child status was: {}", status);

        while let Some(line) = reader.next_line().await? {
            println!("Line: {}", line);
        }

        Ok(())
    }
}


#[cfg(test)]
mod color_test {

    #[test]
    fn test_colored_output(){
        let (r, g, b) = (100, 200, 200);
        let line = "hello colored world";
        println!("\u{1b}[38;2;{r};{g};{b}m{line} \u{1b}[0m", r=r, g=g, b=b, line=line);

    }

    #[test]
    fn test_strfmt() {
        use strfmt::strfmt;
        let template = "python3 {file}";

        let mut vars = std::collections::HashMap::new();
        vars.insert("file".to_string(), "test.py");
        vars.insert("job".to_string(), "python developer");

        let res = strfmt(&template, &vars).unwrap();
        println!("res {}", res)
    }
}


